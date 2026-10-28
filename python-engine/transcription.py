"""Local-only Fun-ASR-Nano orchestration; stdout belongs exclusively to the bridge."""
import json
import ctypes
import math
import re
import subprocess
import sys
import time
import wave
from pathlib import Path

import local_tools
import nano_cuda
import transcript_segments

MODEL_ID = 'fun-asr-nano'
MODEL_REVISION = '46e849502a867080d66d351b8dfb1018b607e509'
VAD_REVISION = '6840bae4c5c92ee8c04faaf4db23dd0105098d7f'
PIPELINE_REVISION = 'fun-asr-nano-cpu-fbbfe018-v1'
MAX_TEXT = 16 * 1024 * 1024
FILES = [('funasr-encoder-f16.gguf', 469331008), ('qwen3-0.6b-q8_0.gguf', 804753280), ('fsmn-vad.gguf', 1720512)]
MANIFEST = dict(model_revision=MODEL_REVISION, vad_revision=VAD_REVISION, pipeline_revision=PIPELINE_REVISION)


class TranscriptionError(Exception):
    pass


def audio_command(ffmpeg, source, audio):
    # HTML media time starts at PTS zero, not necessarily the first packet's PTS.
    return [ffmpeg, '-nostdin', '-hide_banner', '-loglevel', 'error', '-n',
            '-copyts', '-protocol_whitelist', 'file',
            '-format_whitelist', 'mov,matroska,webm', '-i', str(source),
            '-map', '0:a:0', '-vn', '-ac', '1', '-ar', '16000',
            '-af', 'aresample=async=1:first_pts=0', '-c:a', 'pcm_s16le', '-f', 'wav', str(audio)]


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


class RecognitionProgress:
    """Read only native completion signals; never publish native logs or text."""
    def __init__(self, log, total=None, outputs=None):
        self.log, self.outputs = log, outputs
        self.position, self.pending = 0, b''
        self.phase, self.completed, self.total = 'loading_model', 0, total

    def __call__(self):
        if self.log.exists():
            if self.log.stat().st_size > MAX_TEXT:
                raise TranscriptionError('TRANSCRIPTION_OUTPUT_TOO_LARGE')
            with self.log.open('rb') as stream:
                stream.seek(self.position)
                data = stream.read(MAX_TEXT + 1)
                self.position = stream.tell()
            lines = (self.pending + data).split(b'\n')
            self.pending = lines.pop()
            for line in lines:
                if self.outputs is not None:
                    if b' fun_asr_nano.prepare_ms ' in line:
                        self.phase = 'recognizing'
                elif line.startswith(b'YINGQU_ASR_PROGRESS '):
                    match = re.fullmatch(rb'YINGQU_ASR_PROGRESS (segmenting|recognizing) (\d+) (\d+)\r?', line)
                    if not match:
                        raise TranscriptionError('TRANSCRIPTION_FAILED')
                    phase, completed, total = match[1].decode('ascii'), int(match[2]), int(match[3])
                    if phase == 'segmenting':
                        if self.phase != 'loading_model' or completed or total:
                            raise TranscriptionError('TRANSCRIPTION_FAILED')
                    else:
                        if completed > total or completed < self.completed or self.total is not None and total != self.total:
                            raise TranscriptionError('TRANSCRIPTION_FAILED')
                        self.completed, self.total = completed, total
                    self.phase = phase
        if self.outputs is not None:
            while self.completed < self.total:
                result = self.outputs / f'transcript_s{self.completed + 1:06d}.txt'
                if not result.is_file() or result.stat().st_size == 0:
                    break
                # audio.cpp creates this file only after that clip's inference returns.
                self.completed += 1
                self.phase = 'recognizing'
        fields = dict(phase=self.phase)
        if self.total is not None:
            fields.update(completed_segments=self.completed, total_segments=self.total)
        return fields


def run_process(command, output, log, timeout, emit, phase, duration=None, env=None, observe=None):
    started = last = time.monotonic()
    state = dict(phase=phase)
    emit('transcription_progress', **state, duration_seconds=duration)
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
                current = observe() if observe else state
                if current != state or time.monotonic() - last >= 5:
                    state = current
                    emit('transcription_progress', **state, duration_seconds=duration)
                    last = time.monotonic()
                time.sleep(0.1)
            if observe:
                current = observe()
                if current != state:
                    emit('transcription_progress', **current, duration_seconds=duration)
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
        frames = wav.getnframes()
    if not math.isfinite(duration) or duration <= 0:
        raise TranscriptionError('TRANSCRIPTION_AUDIO_FAILED')
    if backend == 'cuda':
        return recognize_cuda(audio, directory, stage, emit, duration, device)
    output, log = stage / 'transcript.srt', stage / 'recognition.log'
    progress = RecognitionProgress(log)
    code = run_process([str(native_program()), '--enc', str(directory / FILES[0][0]),
                        '-m', str(directory / FILES[1][0]),
                        '--vad', str(directory / FILES[2][0]), '--vad-maxseg', '30000', '-a', str(audio), '--srt'],
                       output, log, max(600, 3 * duration + 120), emit, 'loading_model', duration, observe=progress)
    if code:
        raise TranscriptionError('TRANSCRIPTION_FAILED')
    if progress.total is None or progress.completed != progress.total:
        raise TranscriptionError('TRANSCRIPTION_ENGINE_UNAVAILABLE')
    if output.stat().st_size > MAX_TEXT:
        raise TranscriptionError('TRANSCRIPTION_OUTPUT_TOO_LARGE')
    try:
        raw = output.read_text('utf-8-sig').replace('\r\n', '\n').replace('\r', '\n').strip()
        rows = transcript_segments.parse_srt(raw, frames)
        text = '\n'.join(row['text'] for row in rows)
        timing = transcript_segments.write(stage, rows, frames, VAD_REVISION)
    except (UnicodeError, ValueError):
        raise TranscriptionError('TRANSCRIPTION_FAILED') from None
    output = stage / 'transcript.txt'
    output.write_text(text, encoding='utf-8', newline='\n')
    return dict(filename='transcript.txt', bytes=output.stat().st_size, no_speech=not bool(text),
                duration_seconds=duration, backend='cpu', timing=timing)


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
    count, previous_end, windows = 0, 0, []
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
            windows.append((start, min(end, source.getnframes() // 16)))
            count += 1
            source.setpos(first)
            with wave.open(str(clips / f's{count:06d}.wav'), 'wb') as out:
                out.setparams((1, 2, 16000, 0, 'NONE', 'not compressed'))
                out.writeframes(source.readframes(last - first))
    text, rows = '', []
    if count:
        framework_log = stage / 'recognition.framework.log'
        progress = RecognitionProgress(framework_log, count, outputs)
        command = [str(directory / 'audiocpp_cli.exe'), '--task', 'asr', '--family', 'fun_asr_nano',
                   '--model', str(directory / nano_cuda.MODEL_FILE), '--backend', 'cuda', '--device', '0',
                   '--threads', '4', '--language', 'auto', '--max-tokens', '512',
                   '--batch-audio-dir', str(clips), '--audio-chunk-mode', 'none',
                   '--text-out', str(outputs / 'transcript.txt'), '--log-file', str(framework_log)]
        if run_process(command, stage / 'recognition.stdout', stage / 'recognition.log',
                       max(1, deadline - time.monotonic()), emit, 'loading_model', duration, env, observe=progress):
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
            if part == '/sil':
                continue
            parts.append(part)
            start, end = windows[index - 1]
            rows.append(dict(start_ms=start, end_ms=end, text=part))
        text = '\n'.join(parts)
    with wave.open(str(audio), 'rb') as source:
        frames = source.getnframes()
    try:
        timing = transcript_segments.write(stage, rows, frames, VAD_REVISION)
    except ValueError:
        raise TranscriptionError('TRANSCRIPTION_CUDA_FAILED') from None
    output = stage / 'transcript.txt'
    output.write_text(text, encoding='utf-8', newline='\n')
    return dict(filename=output.name, bytes=output.stat().st_size, no_speech=not bool(text),
                duration_seconds=duration, backend='cuda', timing=timing)


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
    code = run_process(audio_command(ffmpeg, source, audio),
                       stage / 'conversion.stdout', stage / 'conversion.log', 1800, emit, 'extracting_audio')
    if code:
        error = (stage / 'conversion.log').read_bytes()[:65536]
        raise TranscriptionError('TRANSCRIPTION_NO_AUDIO' if b'matches no streams' in error
                                 else 'TRANSCRIPTION_AUDIO_FAILED')
    return recognize(audio, model, stage, emit, backend, device)
