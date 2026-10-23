"""Offline startup check of the complete PyInstaller onedir, without Python on PATH."""
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path


engine = Path(sys.argv[1]).resolve(strict=True)
asr = engine.parent / '_internal/asr'
native = asr / ('llama-funasr-cli.exe' if os.name == 'nt' else 'llama-funasr-cli')
for file in [native, asr / ('llama-funasr-vad.exe' if os.name == 'nt' else 'llama-funasr-vad'), asr / 'smoke.wav', asr / 'SOURCE.json', asr / 'FunASR-LICENSE', asr / 'llama.cpp-LICENSE', asr / 'miniaudio-LICENSE', asr / 'models-LICENSE', asr / 'NVIDIA-CUDA-LICENSE']:
    if not file.is_file() or file.stat().st_size == 0:
        raise RuntimeError(f'Missing bundled ASR resource: {file.name}')
if json.loads((asr / 'SOURCE.json').read_text('utf-8')).get('model') != 'Fun-ASR-Nano':
    raise RuntimeError('Bundled ASR helper provenance does not match Fun-ASR-Nano')
if list(asr.glob('*.gguf')):
    raise RuntimeError('Model weights must be downloaded on demand, not bundled')
env = {key: value for key, value in os.environ.items()
       if not key.upper().startswith('PYTHON')}
env['PATH'] = os.path.join(env.get('SystemRoot', 'C:\\Windows'), 'System32') if os.name == 'nt' else '/usr/bin:/bin'
with tempfile.TemporaryDirectory() as cwd:
    result = subprocess.run([str(engine)], input='', capture_output=True, text=True,
                            encoding='utf-8', timeout=30, cwd=cwd, env=env)
if result.returncode != 3:
    raise RuntimeError(f'Engine did not exit on stdin EOF: {result.returncode}: {result.stderr}')
ready = json.loads(result.stdout.strip())
if ready.get('type') != 'ready' or ready.get('protocol_version') != 3:
    raise RuntimeError(f'Invalid engine handshake: {ready}')
print(f"Packaged engine ready: {ready['engine_version']}, yt-dlp {ready['yt_dlp_version']}")
