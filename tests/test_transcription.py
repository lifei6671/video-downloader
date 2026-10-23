import json
import subprocess
import sys
import wave
from pathlib import Path

import pytest
import local_tools
import transcription as asr
import runtime_dependencies as deps


def audio(path):
    with wave.open(str(path), 'wb') as out:
        out.setparams((1, 2, 16000, 0, 'NONE', 'not compressed'))
        out.writeframes(b'\0' * 32000)


def test_normalizes_real_result_and_does_not_put_text_on_bridge(monkeypatch, tmp_path):
    wav = tmp_path / 'audio.wav'; audio(wav)
    seen = []
    def process(command, output, log, timeout, emit, phase, duration=None):
        assert '--backend' not in command and '--enc' in command and '--vad' in command
        assert command[command.index('--enc')+1].endswith('funasr-encoder-f16.gguf')
        assert command[command.index('-m')+1].endswith('qwen3-0.6b-q8_0.gguf')
        assert command[command.index('--vad')+1].endswith('fsmn-vad.gguf')
        assert timeout == 600
        output.write_bytes('\ufeff欢迎体验。\r\n第二行\r\n'.encode())
        emit('transcription_progress', phase=phase, duration_seconds=duration)
        return 0
    monkeypatch.setattr(asr, 'run_process', process)
    result = asr.recognize(wav, tmp_path, tmp_path, lambda kind, **fields: seen.append((kind, fields)))
    assert (tmp_path / result['filename']).read_text('utf-8') == '欢迎体验。\n第二行'
    assert result['bytes'] == len('欢迎体验。\n第二行'.encode())
    assert not result['no_speech'] and 'text' not in result
    assert seen == [('transcription_progress', {'phase':'recognizing', 'duration_seconds':1.0})]


@pytest.mark.parametrize('body,code,error', [(b'',0,None), (b'partial',1,'TRANSCRIPTION_FAILED'), (b'\xff',0,'TRANSCRIPTION_FAILED')])
def test_empty_failure_and_invalid_utf8_are_distinct(monkeypatch, tmp_path, body, code, error):
    wav = tmp_path / 'audio.wav'; audio(wav)
    def process(command, output, *args):
        output.write_bytes(body); return code
    monkeypatch.setattr(asr, 'run_process', process)
    if error:
        with pytest.raises(asr.TranscriptionError, match=error): asr.recognize(wav,tmp_path,tmp_path,lambda *_a,**_k:None)
    else:
        assert asr.recognize(wav,tmp_path,tmp_path,lambda *_a,**_k:None)['no_speech']


def test_model_status_binds_revision_and_rejects_truncated_files(monkeypatch,tmp_path):
    monkeypatch.setattr(asr,'FILES',[('model.gguf',4)])
    (tmp_path/'manifest.json').write_text(json.dumps(asr.MANIFEST))
    (tmp_path/'model.gguf').write_bytes(b'good')
    assert asr.model_available(tmp_path)
    (tmp_path/'model.gguf').write_bytes(b'bad')
    assert not asr.model_available(tmp_path)
    (tmp_path/'model.gguf').write_bytes(b'good')
    (tmp_path/'manifest.json').write_text('{}')
    assert not asr.model_available(tmp_path)


def test_model_redirects_do_not_weaken_tool_downloads():
    request = deps.Request('https://huggingface.co/model')
    model = deps.DownloadRedirects(model=True)
    assert model.redirect_request(request,None,302,'',{},'https://us.aws.cdn.hf.co/file')
    for url in ['http://us.aws.cdn.hf.co/file', 'https://evil.hf.co/file', 'https://127.0.0.1/file']:
        with pytest.raises(deps.DependencyError): model.redirect_request(request,None,302,'',{},url)
    with pytest.raises(deps.DependencyError): deps.DownloadRedirects().redirect_request(request,None,302,'',{},'https://us.aws.cdn.hf.co/file')


def test_timeout_reaps_actual_child(tmp_path):
    with pytest.raises(asr.TranscriptionError,match='TRANSCRIPTION_TIMEOUT'):
        asr.run_process([sys.executable,'-c','import time;time.sleep(10)'],tmp_path/'out',tmp_path/'log',0.1,lambda *_a,**_k:None,'recognizing')


def test_conversion_is_local_only_and_reports_missing_audio(monkeypatch,tmp_path):
    monkeypatch.setattr(local_tools,'MANAGED_DIR',tmp_path)
    source=tmp_path/'中文 视频.mp4'; source.write_bytes(b'original')
    monkeypatch.setattr(asr,'model_available',lambda _:True)
    monkeypatch.setattr(asr,'native_program',lambda:source)
    monkeypatch.setattr(local_tools,'installed_tool',lambda _:'ffmpeg')
    def process(command,output,log,*args):
        assert command[command.index('-protocol_whitelist')+1]=='file'
        assert command[command.index('-i')+1]==str(source)
        assert '0:a:0' in command and 'pcm_s16le' in command
        log.write_bytes(b"Stream map '0:a:0' matches no streams.")
        return 1
    monkeypatch.setattr(asr,'run_process',process)
    with pytest.raises(asr.TranscriptionError,match='TRANSCRIPTION_NO_AUDIO'):
        asr.transcribe(dict(source=str(source),staging_dir=str(tmp_path)),lambda *_a,**_k:None)
    assert source.read_bytes()==b'original'


def test_retired_sensevoice_selection_never_selects_old_models_or_gpu(monkeypatch, tmp_path):
    monkeypatch.setattr(local_tools, 'MANAGED_DIR', tmp_path)
    (tmp_path/'sensevoice').mkdir()
    (tmp_path/'sensevoice-backend.json').write_text('{"variant":"cuda","package":"86"}')
    source=tmp_path/'video.mp4'; source.write_bytes(b'video')
    monkeypatch.setattr(asr, 'native_program', lambda: source)
    def installed(path):
        assert path==tmp_path/'fun-asr-nano'
        return False
    monkeypatch.setattr(asr, 'model_available', installed)
    monkeypatch.setattr(asr, 'run_process', lambda *_: pytest.fail('old model must not be used'))
    with pytest.raises(asr.TranscriptionError, match='TRANSCRIPTION_MODEL_UNAVAILABLE'):
        asr.transcribe(dict(source=str(source),staging_dir=str(tmp_path)),lambda *_a,**_k:None)
    assert json.loads((tmp_path/'sensevoice-backend.json').read_text())['variant']=='cuda'


@pytest.mark.parametrize('platform', ['win32-x64','darwin-x64','darwin-arm64'])
def test_pinned_nano_assets_are_a_complete_independent_model(platform):
    import runtime_assets
    items=runtime_assets.assets(asr.MODEL_ID,platform)
    assert [(row[1],row[3]) for row in items]==asr.FILES
    assert sum(row[3] for row in items)==1275804800
    assert all(len(row[4])==64 for row in items)
    assert runtime_assets.assets('sensevoice',platform)==[]
