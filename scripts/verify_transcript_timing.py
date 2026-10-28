"""WB-301 opt-in real VAD/ASR experiment; inputs read-only, artifacts only in .local."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time
import wave

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'python-engine'))
import transcription
import nano_cuda

RATE = 16000


def digest(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def read_pcm(path):
    with wave.open(str(path), 'rb') as source:
        if (source.getnchannels(), source.getsampwidth(), source.getframerate()) != (1, 2, RATE):
            raise ValueError('Expected mono PCM16 16kHz WAV')
        return source.readframes(source.getnframes())


def write_pcm(path, data):
    with wave.open(str(path), 'wb') as output:
        output.setparams((1, 2, RATE, 0, 'NONE', 'not compressed'))
        output.writeframes(data)


def intervals(text, duration_ms):
    """Validate native millisecond ranges; retain raw EOF overshoot separately."""
    result = []
    previous = 0
    for line in text.splitlines():
        if not re.fullmatch(r'\d+ \d+', line):
            raise ValueError('Invalid VAD interval')
        start, end = map(int, line.split())
        if start < previous or end <= start or end > duration_ms + 30 or start >= duration_ms:
            raise ValueError('Out-of-range or unordered VAD interval')
        result.append((start, min(end, duration_ms)))
        previous = end
    return result


def parse_srt(text):
    pattern = re.compile(r'(\d+):(\d{2}):(\d{2}),(\d{3}) --> (\d+):(\d{2}):(\d{2}),(\d{3})')
    rows = []
    for index, block in enumerate(re.split(r'\n\s*\n', text.strip()), 1):
        if not block:
            continue
        lines = block.splitlines()
        match = pattern.fullmatch(lines[1]) if len(lines) >= 3 else None
        if lines[0] != str(index) or not match:
            raise ValueError('Invalid SRT cue')
        values = list(map(int, match.groups()))
        if any(values[i] >= 60 for i in (1, 2, 5, 6)):
            raise ValueError('Invalid SRT clock')
        stamp = lambda v: ((v[0] * 60 + v[1]) * 60 + v[2]) * 1000 + v[3]
        start, end = stamp(values[:4]), stamp(values[4:])
        if start >= end or rows and start < rows[-1]['end_ms']:
            raise ValueError('Invalid SRT range')
        rows.append(dict(start_ms=start, end_ms=end, text='\n'.join(lines[2:])))
    return rows


def match_cues(cues, windows):
    """SRT can omit /sil; never manufacture ranges for missing ASR output."""
    positions, cursor = [], 0
    for cue in cues:
        target = (cue['start_ms'], cue['end_ms'])
        while cursor < len(windows) and windows[cursor] != target:
            cursor += 1
        if cursor == len(windows):
            raise ValueError('SRT is not an ordered subset of actual VAD windows')
        positions.append(cursor)
        cursor += 1
    return positions


def prepare_cases(output, english, natural):
    zh = read_pcm(ROOT / 'python-engine/assets/sensevoice-smoke.wav')
    en = read_pcm(english)
    # Placement bounds are independently known, NOT human phoneme annotations.
    pieces, blocks, frames = [], [], 0
    for language, data in [('silence', bytes(800 * 32)), ('zh', zh), ('silence', bytes(1800 * 32)),
                           ('en', en), ('silence', bytes(1800 * 32)), ('zh', zh), ('silence', bytes(900 * 32))]:
        first = frames
        pieces.append(data)
        frames += len(data) // 2
        if language != 'silence':
            blocks.append(dict(language=language, start_ms=first // 16, end_ms=frames // 16))
    cases = []
    for name, data, placement in [('short-zh', zh, []), ('pause-zh-en', b''.join(pieces), blocks),
                                   ('continuous-zh', zh * 22, []), ('natural-zh', read_pcm(natural), [])]:
        path = output / f'{name}.wav'
        write_pcm(path, data)
        cases.append(dict(name=name, path=path, duration_ms=len(data) // 32, placement=placement))
    return cases


def run_native(command, output, log, duration, env=None):
    started = time.monotonic()
    with output.open('wb') as stdout, log.open('wb') as stderr:
        result = subprocess.run(command, stdin=subprocess.DEVNULL, stdout=stdout, stderr=stderr,
                                timeout=max(600, duration * 3 + 120), env=env,
                                creationflags=0x08000000 if sys.platform == 'win32' else 0)
    if result.returncode:
        raise RuntimeError(f'Native process failed: {Path(command[0]).name}, exit {result.returncode}')
    return round(time.monotonic() - started, 3)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cpu-directory', required=True, type=Path)
    parser.add_argument('--cuda-directory', type=Path)
    parser.add_argument('--english-audio', required=True, type=Path)
    parser.add_argument('--natural-audio', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path, help='New directory under repository .local')
    args = parser.parse_args()
    args.cpu_directory = args.cpu_directory.resolve()
    if args.cuda_directory:
        args.cuda_directory = args.cuda_directory.resolve()
    output = args.output.resolve()
    if not output.is_relative_to(ROOT / '.local') or output == ROOT / '.local':
        parser.error('Output must be a new directory below repository .local')
    if not transcription.model_available(args.cpu_directory):
        parser.error('Existing validated CPU runtime required; no automatic installation')
    if args.cuda_directory and not nano_cuda.available(args.cuda_directory):
        parser.error('Existing validated CUDA runtime required')
    device = nano_cuda.require_device() if args.cuda_directory else None
    inputs = [args.english_audio, args.natural_audio, ROOT / 'python-engine/assets/sensevoice-smoke.wav']
    before = {str(p.resolve()): digest(p) for p in inputs}
    output.mkdir(parents=True, exist_ok=False)
    summary = dict(status='RUNNING', platform=sys.platform, cases=[], inputs=before,
                   timing_source='FSMN-VAD input window, not sentence/word alignment',
                   cpu_revision=transcription.MANIFEST, cuda_revision=nano_cuda.MANIFEST_ID if args.cuda_directory else None)
    try:
        env = {**os.environ, 'VAD_MAXSEG': '30000'}
        for case in prepare_cases(output, args.english_audio, args.natural_audio):
            name, audio, duration = case['name'], case['path'], case['duration_ms']
            stage = output / name
            stage.mkdir()
            print(f'{name}: VAD and real CPU SRT', flush=True)
            raw_vad = stage / 'vad.txt'
            run_native([str(transcription.vad_program()), '-m', str(args.cpu_directory / 'fsmn-vad.gguf'), '-a', str(audio)],
                       raw_vad, stage / 'vad.log', duration / 1000, env)
            windows = intervals(raw_vad.read_text('utf-8'), duration)
            assert windows, f'{name}: no speech detected'
            srt = stage / 'cpu.srt'
            cpu_seconds = run_native([str(transcription.native_program()), '--enc', str(args.cpu_directory / transcription.FILES[0][0]),
                                     '-m', str(args.cpu_directory / transcription.FILES[1][0]), '--vad', str(args.cpu_directory / 'fsmn-vad.gguf'),
                                     '--vad-maxseg', '30000', '-a', str(audio), '--srt'], srt, stage / 'cpu.log', duration / 1000, env)
            cues = parse_srt(srt.read_text('utf-8-sig').replace('\r\n', '\n'))
            assert cues, f'{name}: no transcript'
            matched = match_cues(cues, windows)
            result = dict(name=name, duration_ms=duration, audio_sha256=digest(audio), vad_windows=windows,
                          cpu_cues=cues, omitted_vad_indices=sorted(set(range(len(windows))) - set(matched)), cpu_seconds=cpu_seconds,
                          longest_window_ms=max(end - start for start, end in windows), placement=case['placement'])
            if case['placement']:
                result['placement_overlap'] = [dict(**block, overlapping_vad_indices=[i for i, (a, b) in enumerate(windows)
                    if a < block['end_ms'] and b > block['start_ms']]) for block in case['placement']]
                assert all(b['overlapping_vad_indices'] for b in result['placement_overlap']), 'Source block missing VAD coverage'
            if name == 'short-zh':
                production = stage / 'production-cpu'
                production.mkdir()
                result['production_cpu_result'] = transcription.recognize(audio, args.cpu_directory, production, lambda *_a, **_k: None)
                assert 'segments' not in result['production_cpu_result']
            if args.cuda_directory:
                print(f'{name}: real production CUDA + preserved stage mapping', flush=True)
                cuda = stage / 'cuda'
                cuda.mkdir()
                events = []
                started = time.monotonic()
                final = transcription.recognize(audio, args.cuda_directory, cuda, lambda event, **fields: events.append(dict(event=event, **fields)), backend='cuda', device=device)
                cuda_windows = intervals((cuda / 'vad.txt').read_text('utf-8'), duration)
                assert cuda_windows == windows, 'CPU/CUDA VAD window mismatch'
                mapped = []
                raw_ends = [int(line.split()[1]) for line in (cuda / 'vad.txt').read_text('utf-8').splitlines()]
                audio_frames = len(read_pcm(audio)) // 2
                for i, (start, end) in enumerate(cuda_windows, 1):
                    clip = cuda / 'clips' / f's{i:06d}.wav'
                    # Native EOF may be a partial millisecond; production clamps in samples.
                    expected_frames = min(raw_ends[i - 1] * 16, audio_frames) - start * 16
                    assert len(read_pcm(clip)) == expected_frames * 2, 'Clip boundary mismatch'
                    text = (cuda / 'recognized' / f'transcript_s{i:06d}.txt').read_text('utf-8-sig').strip()
                    assert text, 'Empty ASR output cannot become timed paragraph'
                    mapped.append(dict(start_ms=start, end_ms=end, text=text))
                assert (cuda / final['filename']).read_text('utf-8') == '\n'.join(row['text'] for row in mapped)
                assert 'segments' not in final
                result.update(cuda_status='PASSED', cuda_seconds=round(time.monotonic() - started, 3),
                              cuda_cues=mapped, production_cuda_result=final, events=events)
            else:
                result['cuda_status'] = 'NOT_RUN'
            summary['cases'].append(result)
            (stage / 'result.json').write_text(json.dumps(result, ensure_ascii=False, indent=2), encoding='utf-8')
            print(f'{name}: PASSED, {len(windows)} VAD / {len(cues)} CPU cues', flush=True)
        assert all(digest(path) == expected for path, expected in before.items()), 'Source audio changed'
        summary['status'] = 'PASSED'
    except Exception as error:
        summary.update(status='FAILED', error=f'{type(error).__name__}: {error}')
        raise
    finally:
        (output / 'summary.json').write_text(json.dumps(summary, ensure_ascii=False, indent=2), encoding='utf-8')


if __name__ == '__main__':
    main()
