"""Local-only Fun-ASR-Nano orchestration; stdout belongs exclusively to the bridge."""
import json
import ctypes
import math
import subprocess
import sys
import time
import wave
from pathlib import Path

import local_tools
import nano_cuda

MODEL_ID = 'fun-asr-nano'
MODEL_REVISION = '46e849502a867080d66d351b8dfb1018b607e509'
VAD_REVISION = '6840bae4c5c92ee8c04faaf4db23dd0105098d7f'
PIPELINE_REVISION = 'fun-asr-nano-cpu-fbbfe018-v1'
MAX_TEXT = 16 * 1024 * 1024
FILES = [('funasr-encoder-f16.gguf', 469331008), ('qwen3-0.6b-q8_0.gguf', 804753280), ('fsmn-vad.gguf', 1720512)]
MANIFEST = dict(model_revision=MODEL_REVISION, vad_revision=VAD_REVISION, pipeline_revision=PIPELINE_REVISION)


class TranscriptionError(Exception):
    pass


def resources():
    return Path(getattr(sys, '_MEIPASS', Path(__file__).resolve().parent)) / 'asr'


def native_program():
    return resources() / ('llama-funasr-cli.exe' if sys.platform == 'win32' else 'llama-funasr-cli')


def vad_program():
    return resources() / ('llama-funasr-vad.exe' if sys.platform == 'win32' else 'llama-funasr-vad')


def model_available(directory):
    try:
        root = Path(directory)
        return (not root.is_symlink() and json.loads((root / 'manifest.json').read_text('utf-8')) == MANIFEST
                and all(not (root / name).is_symlink() and (root / name).stat().st_size == size for name, size in FILES))
    except (OSError, ValueError):
        return False


def run_process(command, output, log, timeout, emit, phase, duration=None, env=None):
    started = last = time.monotonic()
    emit('transcription_progress', phase=phase, duration_seconds=duration)
    with output.open('wb') as out, log.open('wb') as err:
        foreign_dlls = sys.platform == 'win32' and hasattr(sys, '_MEIPASS') and Path(command[0]).name == 'audiocpp_cli.exe'
        if foreign_dlls:
            ctypes.windll.kernel32.SetDllDirectoryW(None)
        try:
            child = subprocess.Popen(command, stdin=subprocess.DEVNULL, stdout=out, stderr=err, env=env,
                                     creationflags=0x08000000 if sys.platform == 'win32' else 0)
        finally:
            if foreign_dlls:
                ctypes.windll.kernel32.SetDllDirectoryW(str(sys._MEIPASS))
        try:
            while child.poll() is None:
                if time.monotonic() - started > timeout:
                    raise TranscriptionError('TRANSCRIPTION_TIMEOUT')
                if output.stat().st_size > MAX_TEXT or log.stat().st_size > MAX_TEXT:
                    raise TranscriptionError('TRANSCRIPTION_OUTPUT_TOO_LARGE')
                if time.monotonic() - last >= 5:
                    emit('transcription_progress', phase=phase, duration_seconds=duration)
                    last = time.monotonic()
                time.sleep(0.1)
            return child.returncode
        finally:
            if child.poll() is None:
                child.kill()
            child.wait()


def recognize(audio, directory, stage, emit, backend='cpu', device=None):
    with wave.open(str(audio), 'rb') as wav:
        if wav.getframerate() != 16000 or wav.getnchannels() != 1 or wav.getsampwidth() != 2:
            raise TranscriptionError('TRANSCRIPTION_AUDIO_FAILED')
        duration = wav.getnframes() / 16000
    if not math.isfinite(duration) or duration <= 0:
        raise TranscriptionError('TRANSCRIPTION_AUDIO_FAILED')
    if backend == 'cuda':
        return recognize_cuda(audio, directory, stage, emit, duration, device)
    output, log = stage / 'transcript.txt', stage / 'recognition.log'
    code = run_process([str(native_program()), '--enc', str(directory / FILES[0][0]),
                        '-m', str(directory / FILES[1][0]),
                        '--vad', str(directory / FILES[2][0]), '--vad-maxseg', '30000', '-a', str(audio)],
                       output, log, max(600, 3 * duration + 120), emit, 'recognizing', duration)
    if code:
        raise TranscriptionError('TRANSCRIPTION_FAILED')
    if output.stat().st_size > MAX_TEXT:
        raise TranscriptionError('TRANSCRIPTION_OUTPUT_TOO_LARGE')
    try:
        text = output.read_text('utf-8-sig').replace('\r\n', '\n').replace('\r', '\n').strip()
    except UnicodeError:
        raise TranscriptionError('TRANSCRIPTION_FAILED') from None
    output.write_text(text, encoding='utf-8', newline='\n')
    return dict(filename='transcript.txt', bytes=output.stat().st_size, no_speech=not bool(text),
                duration_seconds=duration, backend='cpu')


def recognize_cuda(audio, directory, stage, emit, duration, device):
    """Detect speech without ASR, then run all clips in one loaded CUDA session."""
    env = nano_cuda.environment(directory, device)
    deadline = time.monotonic() + max(600, 3 * duration + 120)
    intervals, log = stage / 'vad.txt', stage / 'vad.log'
    if run_process([str(vad_program()), '-m', str(directory / nano_cuda.VAD_FILE), '-a', str(audio)],
                   intervals, log, max(1, deadline - time.monotonic()), emit, 'segmenting', duration, env):
        raise TranscriptionError('TRANSCRIPTION_VAD_FAILED')
    if intervals.stat().st_size > MAX_TEXT:
        raise TranscriptionError('TRANSCRIPTION_OUTPUT_TOO_LARGE')
    clips, outputs = stage / 'clips', stage / 'recognized'
    clips.mkdir(); outputs.mkdir()
    count, previous_end = 0, 0
    with wave.open(str(audio), 'rb') as source:
        for line in intervals.read_text('utf-8').splitlines():
            try:
                start, end = map(int, line.split())
            except ValueError:
                raise TranscriptionError('TRANSCRIPTION_VAD_FAILED') from None
            if start < previous_end or end <= start or end > duration * 1000 + 30:
                raise TranscriptionError('TRANSCRIPTION_VAD_FAILED')
            first, last = start * 16, min(end * 16, source.getnframes())
            if last <= first:
                raise TranscriptionError('TRANSCRIPTION_VAD_FAILED')
            previous_end = end
            count += 1
            source.setpos(first)
            with wave.open(str(clips / f's{count:06d}.wav'), 'wb') as out:
                out.setparams((1, 2, 16000, 0, 'NONE', 'not compressed'))
                out.writeframes(source.readframes(last - first))
    text = ''
    if count:
        command = [str(directory / 'audiocpp_cli.exe'), '--task', 'asr', '--family', 'fun_asr_nano',
                   '--model', str(directory / nano_cuda.MODEL_FILE), '--backend', 'cuda', '--device', '0',
                   '--threads', '4', '--language', 'auto', '--max-tokens', '512',
                   '--batch-audio-dir', str(clips), '--audio-chunk-mode', 'none',
                   '--text-out', str(outputs / 'transcript.txt')]
        if run_process(command, stage / 'recognition.stdout', stage / 'recognition.log',
                       max(1, deadline - time.monotonic()), emit, 'recognizing', duration, env):
            raise TranscriptionError('TRANSCRIPTION_CUDA_FAILED')
        parts, size = [], 0
        for index in range(1, count + 1):
            path = outputs / f'transcript_s{index:06d}.txt'
            try:
                size += path.stat().st_size
                if size > MAX_TEXT:
                    raise TranscriptionError('TRANSCRIPTION_OUTPUT_TOO_LARGE')
                part = path.read_text('utf-8-sig').strip()
            except (OSError, UnicodeError):
                raise TranscriptionError('TRANSCRIPTION_CUDA_FAILED') from None
            # A positive VAD clip without a result must not silently disappear.
            if not part:
                raise TranscriptionError('TRANSCRIPTION_CUDA_FAILED')
            parts.append(part)
        text = '\n'.join(parts)
    output = stage / 'transcript.txt'
    output.write_text(text, encoding='utf-8', newline='\n')
    return dict(filename=output.name, bytes=output.stat().st_size, no_speech=not bool(text),
                duration_seconds=duration, backend='cuda')


def smoke(directory, stage, emit, backend='cpu', device=None):
    if not native_program().is_file():
        raise TranscriptionError('TRANSCRIPTION_ENGINE_UNAVAILABLE')
    if backend == 'cuda' and not vad_program().is_file():
        raise TranscriptionError('TRANSCRIPTION_ENGINE_UNAVAILABLE')
    result = recognize(resources() / 'smoke.wav', directory, stage, lambda *_a, **_k: emit(), backend, device)
    if result['no_speech']:
        raise TranscriptionError('DEPENDENCY_VALIDATION_FAILED')


def transcribe(params, emit):
    source, stage = Path(params['source']), Path(params['staging_dir'])
    if not source.is_absolute() or not source.is_file() or not stage.is_absolute() or not stage.is_dir() or stage.is_symlink():
        raise TranscriptionError('FILE_MISSING')
    backend = nano_cuda.selected()
    model = nano_cuda.directory() if backend == 'cuda' else local_tools.MANAGED_DIR / MODEL_ID
    if backend is None or not (nano_cuda.available(model) if backend == 'cuda' else model_available(model)):
        raise TranscriptionError('TRANSCRIPTION_MODEL_UNAVAILABLE')
    if not native_program().is_file():
        raise TranscriptionError('TRANSCRIPTION_ENGINE_UNAVAILABLE')
    if backend == 'cuda' and not vad_program().is_file():
        raise TranscriptionError('TRANSCRIPTION_ENGINE_UNAVAILABLE')
    device = nano_cuda.require_device() if backend == 'cuda' else None
    send = emit
    def emit(kind, **fields):
        send(kind, backend=backend, **fields)
    ffmpeg = local_tools.installed_tool('ffmpeg')
    if not ffmpeg:
        raise TranscriptionError('FFMPEG_UNAVAILABLE')
    audio = stage / 'audio.wav'
    code = run_process([ffmpeg, '-nostdin', '-hide_banner', '-loglevel', 'error', '-n',
                        '-protocol_whitelist', 'file', '-format_whitelist', 'mov,matroska,webm',
                        '-i', str(source), '-map', '0:a:0', '-vn', '-ac', '1', '-ar', '16000',
                        '-c:a', 'pcm_s16le', '-f', 'wav', str(audio)],
                       stage / 'conversion.stdout', stage / 'conversion.log', 1800, emit, 'extracting_audio')
    if code:
        error = (stage / 'conversion.log').read_bytes()[:65536]
        raise TranscriptionError('TRANSCRIPTION_NO_AUDIO' if b'matches no streams' in error
                                 else 'TRANSCRIPTION_AUDIO_FAILED')
    return recognize(audio, model, stage, emit, backend, device)
