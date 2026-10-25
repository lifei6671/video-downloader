"""Regression coverage for independent VAD, CUDA selection and atomic installation."""
import hashlib
import io
import json
import sys
import wave
import zipfile

import pytest

import local_tools
import nano_cuda as cuda
import runtime_dependencies as deps
import transcription as asr
from test_transcription import audio


@pytest.mark.parametrize('rows,reason,device', [
    ([], 'driver_unavailable', None),
    ([dict(compute='8.9', driver=12030, device='GPU-a')], 'driver_too_old', 'GPU-a'),
    ([dict(compute='12.0', driver=13020, device='GPU-a')], 'unsupported_gpu', None),
    ([dict(compute='6.1', driver=12040, device='GPU-a'), dict(compute='8.9', driver=12040, device='GPU-b')], None, 'GPU-b'),
])
def test_cuda_driver_and_multigpu_boundary(monkeypatch, rows, reason, device):
    monkeypatch.setattr(cuda.sys, 'platform', 'win32')
    monkeypatch.setattr(cuda.platform, 'machine', lambda: 'AMD64')
    monkeypatch.setattr(cuda, 'devices', lambda: rows)
    result = cuda.detect()
    assert result['reason'] == reason and result.get('device') == device


@pytest.mark.parametrize('platform,arch', [('darwin', 'arm64'), ('darwin', 'x86_64'), ('win32', 'ARM64')])
def test_unsupported_platform_never_loads_driver(monkeypatch, platform, arch):
    monkeypatch.setattr(cuda.sys, 'platform', platform)
    monkeypatch.setattr(cuda.platform, 'machine', lambda: arch)
    monkeypatch.setattr(cuda, 'devices', lambda: pytest.fail('driver must not be queried'))
    assert cuda.detect()['reason'] == 'unsupported_system'


def test_default_and_retired_selection_and_status_never_probe_gpu(monkeypatch, tmp_path):
    monkeypatch.setattr(local_tools, 'MANAGED_DIR', tmp_path)
    monkeypatch.setattr(cuda, 'devices', lambda: pytest.fail('ordinary detection must not query GPU'))
    monkeypatch.setattr(local_tools, 'installed_tool', lambda _: None)
    monkeypatch.setattr(local_tools, 'javascript_runtime', lambda: {})
    (tmp_path / 'sensevoice-backend.json').write_text('{"variant":"cuda"}')
    assert cuda.selected() == 'cpu'
    assert deps.status()['tools'][-1]['provider'] == 'cpu'
    cuda.selection_file().write_text('invalid')
    assert cuda.selected() is None and not deps.status()['tools'][-1]['available']


def test_incompatible_cuda_rejected_before_network_or_changes(monkeypatch, tmp_path):
    monkeypatch.setattr(local_tools, 'MANAGED_DIR', tmp_path)
    monkeypatch.setattr(cuda, 'detect', lambda: dict(reason='driver_unavailable'))
    monkeypatch.setattr(deps, 'build_opener', lambda *_: pytest.fail('no download before compatibility'))
    with pytest.raises(deps.DependencyError, match='CUDA_DRIVER_UNAVAILABLE'):
        deps.install(asr.MODEL_ID, tmp_path, lambda *_a, **_k: None, 'cuda')
    assert not list(tmp_path.iterdir())


@pytest.mark.parametrize('variant,tool', [('metal', 'fun-asr-nano'), ('cuda', 'ffmpeg'), ('cpu', 'javascript')])
def test_invalid_variant_request_rejected(tmp_path, variant, tool):
    with pytest.raises(deps.DependencyError, match='ENGINE_PROTOCOL_ERROR'):
        deps.install(tool, tmp_path, lambda *_a, **_k: None, variant)


@pytest.mark.parametrize('intervals,error', [('0 400\n600 1000\n', None), ('', None),
    ('0 1000\n900 1000\n', 'TRANSCRIPTION_VAD_FAILED'), ('-1 500', 'TRANSCRIPTION_VAD_FAILED'),
    ('0 2000', 'TRANSCRIPTION_VAD_FAILED'), ('nonsense', 'TRANSCRIPTION_VAD_FAILED')])
def test_independent_vad_slices_original_pcm_and_uses_one_cuda_session(monkeypatch, tmp_path, intervals, error):
    wav = tmp_path / 'audio.wav'
    pcm = bytes(range(256)) * 125
    with wave.open(str(wav), 'wb') as out:
        out.setparams((1, 2, 16000, 0, 'NONE', 'not compressed')); out.writeframes(pcm)
    calls = []
    def run(command, output, log, timeout, emit, phase, duration, env, observe=None):
        calls.append(command)
        assert env['CUDA_VISIBLE_DEVICES'] == 'GPU-test' and env['VAD_MAXSEG'] == '30000'
        if phase == 'segmenting':
            assert '--enc' not in command and 'llama-funasr-vad' in command[0]
            output.write_text(intervals)
        else:
            assert command[command.index('--backend')+1] == 'cuda'
            assert command[command.index('--audio-chunk-mode')+1] == 'none'
            assert '--batch-audio-dir' in command
            clips = sorted((tmp_path / 'clips').glob('*.wav'))
            assert len(clips) == 2
            for i, (clip, start, end) in enumerate(zip(clips, [0,600], [400,1000]), 1):
                with wave.open(str(clip), 'rb') as part:
                    assert part.readframes(part.getnframes()) == pcm[start*32:end*32]
                (tmp_path / 'recognized' / f'transcript_s{i:06d}.txt').write_text(['第一句。', '第二句。'][i-1], encoding='utf-8')
        return 0
    monkeypatch.setattr(asr, 'run_process', run)
    if error:
        with pytest.raises(asr.TranscriptionError, match=error):
            asr.recognize(wav, tmp_path, tmp_path, lambda *_a, **_k: None, 'cuda', 'GPU-test')
    else:
        result = asr.recognize(wav, tmp_path, tmp_path, lambda *_a, **_k: None, 'cuda', 'GPU-test')
        assert result['backend'] == 'cuda'
        assert result['no_speech'] == (not intervals)
        assert len(calls) == (2 if intervals else 1)
        assert (tmp_path / 'transcript.txt').read_text('utf-8') == ('第一句。\n第二句。' if intervals else '')


@pytest.mark.parametrize('mode', ['empty', 'missing', 'invalid', 'exit'])
def test_positive_vad_without_valid_gpu_result_never_silently_omits_segment(monkeypatch, tmp_path, mode):
    wav = tmp_path / 'audio.wav'; audio(wav)
    def run(command, output, log, timeout, emit, phase, duration, env, observe=None):
        if phase == 'segmenting': output.write_text('0 1000\n')
        elif mode == 'exit': return 1
        elif mode != 'missing':
            (tmp_path / 'recognized' / 'transcript_s000001.txt').write_bytes(b'\xff' if mode == 'invalid' else b'')
        return 0
    monkeypatch.setattr(asr, 'run_process', run)
    with pytest.raises(asr.TranscriptionError, match='TRANSCRIPTION_CUDA_FAILED'):
        asr.recognize(wav, tmp_path, tmp_path, lambda *_a, **_k: None, 'cuda', 'GPU-test')
    assert not (tmp_path / 'transcript.txt').exists()


@pytest.mark.parametrize('failure', [None, 'checksum', 'load', 'missing-dll', 'post-publish'])
def test_cuda_install_and_switch_is_atomic_preserves_cpu_and_reuses_vad(monkeypatch, tmp_path, failure):
    monkeypatch.setattr(local_tools, 'MANAGED_DIR', tmp_path)
    monkeypatch.setattr(deps, 'platform_key', lambda: 'win32-x64')
    monkeypatch.setattr(cuda, 'require_device', lambda: 'GPU-test')
    monkeypatch.setattr(local_tools, 'installed_tool', lambda _: None)
    monkeypatch.setattr(local_tools, 'javascript_runtime', lambda: {})
    resources = tmp_path / 'resources'; resources.mkdir()
    for name in ['native', 'models-LICENSE', 'NVIDIA-CUDA-LICENSE']:
        (resources / name).write_bytes(b'fixture')
    monkeypatch.setattr(asr, 'resources', lambda: resources)
    monkeypatch.setattr(asr, 'native_program', lambda: resources / 'native')
    monkeypatch.setattr(asr, 'vad_program', lambda: resources / 'native')
    monkeypatch.setattr(cuda, 'MODEL_SIZE', 4); monkeypatch.setattr(cuda, 'VAD_SIZE', 4)
    monkeypatch.setattr(asr, 'FILES', [('encoder',4), ('decoder',4), (cuda.VAD_FILE,4)])
    digest = hashlib.sha256(b'good').hexdigest()
    cpu_assets = [(f'https://fixture/{n}',n,'raw',s,digest) for n,s in asr.FILES]
    monkeypatch.setattr(deps, 'assets', lambda tool, _: cpu_assets if tool == asr.MODEL_ID else [])
    cpu = tmp_path / asr.MODEL_ID; cpu.mkdir()
    for name, _ in asr.FILES: (cpu / name).write_bytes(b'good')
    (cpu / 'manifest.json').write_text(json.dumps(asr.MANIFEST))
    (cpu / 'keep').write_bytes(b'original')
    downloads, bodies = [], {}
    for filename, compression, members in [('native.zip','nano-native',cuda.NATIVE_FILES), ('cuda.zip','nano-cudart',cuda.CUDA_FILES)]:
        stream = io.BytesIO()
        with zipfile.ZipFile(stream, 'w') as z:
            for name in members:
                if failure != 'missing-dll' or name != 'cublas64_12.dll': z.writestr(name, b'fixture')
            z.writestr('../escape', b'bad')
        body = stream.getvalue(); url = 'https://fixture/' + filename
        bodies[url] = body
        downloads.append((url,filename,compression,len(body),hashlib.sha256(body).hexdigest()))
    downloads.append(('https://fixture/model',cuda.MODEL_FILE,'raw',4,digest)); bodies[downloads[-1][0]] = b'good'
    monkeypatch.setattr(cuda, 'downloads', lambda: downloads)
    requested = []
    class Response(io.BytesIO): status = 200
    class Opener:
        def open(self, request, timeout):
            requested.append(request.full_url)
            assert request.full_url in bodies  # Existing VAD must not be downloaded.
            return Response(b'bad' if failure == 'checksum' else bodies[request.full_url])
    monkeypatch.setattr(deps, 'build_opener', lambda *_: Opener())
    def smoke(directory, stage, emit, backend='cpu', device=None):
        assert backend == 'cpu' or device == 'GPU-test'
        if failure == 'load': raise asr.TranscriptionError('TRANSCRIPTION_CUDA_FAILED')
    monkeypatch.setattr(asr, 'smoke', smoke)
    if failure == 'post-publish': monkeypatch.setattr(cuda, 'available', lambda *_: False)
    stage = tmp_path / '.install'; stage.mkdir()
    if failure:
        with pytest.raises(deps.DependencyError): deps.install(asr.MODEL_ID, stage, lambda *_a, **_k: None, 'cuda')
        assert cuda.selected() == 'cpu' and not cuda.directory().exists()
    else:
        result = deps.install(asr.MODEL_ID, stage, lambda *_a, **_k: None, 'cuda')
        assert result['tools'][-1]['provider'] == 'cuda' and result['tools'][-1]['available']
        assert cuda.available() and len(requested) == 3
        monkeypatch.setattr(deps, 'build_opener', lambda *_: pytest.fail('switch must not redownload'))
        for variant in ['cpu', 'cuda']:
            switch = tmp_path / ('.switch-' + variant); switch.mkdir()
            result = deps.install(asr.MODEL_ID, switch, lambda *_a, **_k: None, variant)
            assert result['tools'][-1]['provider'] == variant and result['tools'][-1]['available']
    assert (cpu / 'keep').read_bytes() == b'original'
    assert not (tmp_path / 'escape').exists()
