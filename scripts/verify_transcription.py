"""Opt-in CPU/CUDA verification in .local; never uses platform credentials."""
import argparse
import ctypes
import hashlib
import json
import os
import queue
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'python-engine'))
import local_tools
import runtime_assets
import runtime_dependencies
import transcription
import nano_cuda
from urllib.parse import urlsplit


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--download-models', action='store_true')
    parser.add_argument('--long-seconds', type=int, default=0)
    parser.add_argument('--engine', type=Path, help='Also exercise the packaged JSONL bridge')
    parser.add_argument('--variant', choices=['cpu', 'cuda'], default='cpu')
    parser.add_argument('--runtime-directory', type=Path, help='Isolated test runtime, never the user application directory')
    parser.add_argument('--source', type=Path, help='Existing local video instead of generating the smoke video')
    parser.add_argument('--asset-cache', type=Path, help='Read fixed downloads from a local cache; installation still validates all hashes')
    args = parser.parse_args()
    root = ROOT / '.local/nano-verification'
    root.mkdir(parents=True, exist_ok=True)
    runtime = (args.runtime_directory or root / 'runtime').resolve()
    if not runtime.is_relative_to(ROOT / '.local'):
        raise RuntimeError('Verification runtime must be under repository .local')
    runtime.mkdir(parents=True, exist_ok=True)
    runtime_dependencies.configure(str(runtime))
    if args.download_models:
        if args.asset_cache:
            cache = args.asset_cache.resolve()
            class CachedOpener:
                def open(self, request, timeout):
                    path = cache / Path(urlsplit(request.full_url).path).name
                    if not path.is_file():
                        raise RuntimeError(f'Fixed asset missing in cache: {path.name}')
                    class Response:
                        status = 200
                        def __enter__(self): self.file = path.open('rb'); return self
                        def __exit__(self, *_): self.file.close()
                        def read(self, count): return self.file.read(count)
                    return Response()
            runtime_dependencies.build_opener = lambda *_: CachedOpener()
        with tempfile.TemporaryDirectory(prefix='.install-', dir=runtime) as stage:
            runtime_dependencies.install(transcription.MODEL_ID, stage, lambda kind, **v: print(kind, v['phase'], flush=True), args.variant)
    installed = nano_cuda.available() if args.variant == 'cuda' else transcription.model_available(runtime / transcription.MODEL_ID)
    if not installed or nano_cuda.selected() != args.variant:
        raise RuntimeError('Run with --download-models first')
    model_root = nano_cuda.directory() if args.variant == 'cuda' else runtime / transcription.MODEL_ID
    model_assets = nano_cuda.downloads()[2:] + runtime_assets.assets(transcription.MODEL_ID, runtime_dependencies.platform_key())[-1:] if args.variant == 'cuda' else runtime_assets.assets(transcription.MODEL_ID, runtime_dependencies.platform_key())
    for _url, name, _kind, size, digest in model_assets:
        with (model_root / name).open('rb') as stream:
            assert hashlib.file_digest(stream, 'sha256').hexdigest() == digest
        assert (model_root / name).stat().st_size == size
    ffmpeg = local_tools.installed_tool('ffmpeg')
    if not ffmpeg:
        raise RuntimeError('An existing FFmpeg is required')
    video = args.source.resolve() if args.source else root / f'语音 视频-{args.long_seconds or 3}.mkv'
    # Keep the reference speech lossless so codec changes do not alter the ASR fixture.
    if not args.source:
        subprocess.run([ffmpeg, '-nostdin', '-y', '-stream_loop', '-1', '-i', str(ROOT / 'python-engine/assets/sensevoice-smoke.wav'),
                    '-f', 'lavfi', '-i', 'color=size=160x90:rate=1', '-map', '1:v:0', '-map', '0:a:0',
                    '-t', str(args.long_seconds or 3), '-c:v', 'libx264', '-c:a', 'pcm_s16le', str(video)],
                   check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    before = hashlib.sha256(video.read_bytes()).hexdigest()
    peaks = []
    original_popen = subprocess.Popen
    class ObservedProcess(original_popen):
        def __init__(self, command, *a, **kw):
            super().__init__(command, *a, **kw)
            peak = {'program': Path(command[0]).name, 'peak_working_set_bytes': None}
            peaks.append(peak)
            if os.name == 'nt':
                def monitor():
                    from ctypes import wintypes
                    class Counters(ctypes.Structure):
                        _fields_ = [('cb',wintypes.DWORD),('PageFaultCount',wintypes.DWORD)] + [(n,ctypes.c_size_t) for n in ['PeakWorkingSetSize','WorkingSetSize','QuotaPeakPagedPoolUsage','QuotaPagedPoolUsage','QuotaPeakNonPagedPoolUsage','QuotaNonPagedPoolUsage','PagefileUsage','PeakPagefileUsage']]
                    get = ctypes.windll.psapi.GetProcessMemoryInfo
                    get.argtypes = [wintypes.HANDLE, ctypes.POINTER(Counters), wintypes.DWORD]
                    while self.poll() is None:
                        counters = Counters(); counters.cb = ctypes.sizeof(counters)
                        if get(int(self._handle), ctypes.byref(counters), counters.cb):
                            peak['peak_working_set_bytes'] = max(peak['peak_working_set_bytes'] or 0, counters.PeakWorkingSetSize)
                        time.sleep(0.05)
                threading.Thread(target=monitor, daemon=True).start()
    subprocess.Popen = ObservedProcess
    started = time.monotonic()
    with tempfile.TemporaryDirectory(prefix='转录-', dir=root) as stage:
        result = transcription.transcribe(dict(source=str(video), staging_dir=stage), lambda kind, **v: print(kind, v, flush=True))
        text = (Path(stage) / result['filename']).read_text('utf-8')
        assert text and (args.source or ('开放时间' in text and ('9点' in text or '九点' in text))), text
        (root / f'{args.variant}-transcript.txt').write_text(text, encoding='utf-8')
        report = dict(backend=result['backend'], model='Fun-ASR-Nano',
                      duration_seconds=result['duration_seconds'], elapsed_seconds=time.monotonic()-started,
                      text=text, processes=peaks, source_unchanged=before == hashlib.sha256(video.read_bytes()).hexdigest())
        assert report['source_unchanged']
    subprocess.Popen = original_popen
    if args.engine:
        with tempfile.TemporaryDirectory(prefix='packaged-', dir=root) as stage:
            child = subprocess.Popen([str(args.engine.resolve())], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
            messages = queue.Queue()
            def read():
                for line in child.stdout: messages.put(json.loads(line))
                messages.put(None)
            threading.Thread(target=read, daemon=True).start()
            try:
                assert messages.get(timeout=30)['type'] == 'ready'
                request = dict(protocol_version=3,id='real-asr',method='transcribe_video',params=dict(source=str(video),staging_dir=stage,runtime_directory=str(runtime)))
                child.stdin.write((json.dumps(request)+'\n').encode()); child.stdin.flush()
                while True:
                    event = messages.get(timeout=70)
                    if event['type'] == 'transcription_progress': continue
                    assert event['type'] == 'transcribed', event
                    assert event['backend'] == result['backend']
                    assert (Path(stage) / event['filename']).read_text('utf-8') == text
                    break
                assert messages.get(timeout=10) is None
                assert child.wait(timeout=5) == 0
                report['packaged_bridge'] = 'PASS'
            finally:
                if child.poll() is None: child.kill()
                child.wait()
                child.stdin.close(); child.stdout.close()
    name = f'{args.variant}-source-result.json' if args.source else f'{args.variant}-long-result.json' if args.long_seconds else f'{args.variant}-result.json'
    (root / name).write_text(json.dumps(report,ensure_ascii=False,indent=2),encoding='utf-8')
    print(json.dumps({**report,'text':text[:120]},ensure_ascii=False),flush=True)


if __name__ == '__main__':
    main()
