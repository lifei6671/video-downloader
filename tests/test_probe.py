import json
import os
import subprocess
import sys
from pathlib import Path

import pytest
from conftest import BODY

ROOT = Path(__file__).resolve().parents[1]
PROBE = ROOT / 'src-tauri/target/debug' / ('m1-probe.exe' if os.name == 'nt' else 'm1-probe')
FIXTURE = ROOT / 'tests/support_engine.py'


def config(tmp_path, proxy):
    return {'inputs':['https://www.tiktok.com/@u/video/123'],
            'profiles':{'tiktok':{'proxy':proxy,'cookie':'session=must-not-leak'}},'output_dir':str(tmp_path)}


@pytest.mark.parametrize('mode', ['good','unknown'])
def test_actual_downloader_to_rust_finalize(mode, tmp_path, media_proxy):
    proxy, requests, _, _ = media_proxy
    env = {**os.environ,'FIXTURE_MODE':mode}
    run = lambda: subprocess.run([str(PROBE),sys.executable,str(FIXTURE)],
        input=json.dumps(config(tmp_path,proxy)).encode(),capture_output=True,env=env,timeout=15)
    result = run()
    assert result.returncode == 0, (result.stdout,result.stderr)
    assert b'completed' in result.stdout and b'must-not-leak' not in result.stdout + result.stderr
    assert (tmp_path/'tiktok_123.mp4').read_bytes() == BODY
    assert not any('Cookie' in headers for _,headers in requests)
    assert list(tmp_path.iterdir()) == [tmp_path/'tiktok_123.mp4']
    second = run()
    assert second.returncode == 1
    assert b'completed' not in second.stdout
    assert (tmp_path/'tiktok_123.mp4').read_bytes() == BODY


@pytest.mark.parametrize('mode',['html','truncated','redirect','segmented','playlist'])
def test_rejects_bad_media_and_cleans_staging(mode,tmp_path,media_proxy):
    proxy, requests, _, _ = media_proxy
    result=subprocess.run([str(PROBE),sys.executable,str(FIXTURE)],
        input=json.dumps(config(tmp_path,proxy)).encode(),capture_output=True,
        env={**os.environ,'FIXTURE_MODE':mode},timeout=15)
    assert result.returncode == 1
    assert b'completed' not in result.stdout
    assert list(tmp_path.iterdir()) == []
    if mode == 'redirect':
        assert len(requests) == 1  # malicious redirect never reaches the proxy


@pytest.mark.parametrize('mode',['bad_ready','wrong_version','wrong_id','eof','duplicate','nonzero','secret_error'])
def test_bridge_stops_bad_engine(mode,tmp_path):
    script=tmp_path/'fake.py'
    script.write_text('''import json,sys
mode=sys.argv[1]
print(json.dumps({'type':'ready','protocol_version':1 if mode=='wrong_version' else 2,
 'engine_version':'bad' if mode=='bad_ready' else '0.2.1','yt_dlp_version':'2026.08.19'}),flush=True)
request=json.loads(sys.stdin.readline())
if mode=='eof': sys.exit(0)
if mode=='secret_error':
 print(json.dumps({'protocol_version':2,'id':request['id'],'type':'error','error':{'code':'session=private','message':'secret'}}),flush=True)
 sys.exit(1)
msg={'protocol_version':2,'id':'wrong' if mode=='wrong_id' else request['id'],'type':'resolved','result':{'platform':'tiktok','video_id':'123','canonical_url':'https://www.tiktok.com/@u/video/123'}}
print(json.dumps(msg),flush=True)
if mode=='duplicate': print(json.dumps(msg),flush=True)
if mode=='nonzero': sys.exit(2)
if mode=='wrong_id': sys.stdin.read()
''')
    result=subprocess.run([str(PROBE),sys.executable,str(script),mode],
        input=json.dumps(config(tmp_path,None)).encode(),capture_output=True,timeout=20)
    assert result.returncode == 1
    assert b'ENGINE_PROTOCOL_ERROR' in result.stdout
    assert b'completed' not in result.stdout
    assert b'private' not in result.stdout + result.stderr


def test_eof_cancels_blocked_network(tmp_path,media_proxy):
    proxy, _, started, _ = media_proxy
    stage=tmp_path/'stage'; stage.mkdir()
    process=subprocess.Popen([sys.executable,str(FIXTURE)],stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,stderr=subprocess.PIPE,env={**os.environ,'FIXTURE_MODE':'stall'})
    assert json.loads(process.stdout.readline())['type']=='ready'
    request={'protocol_version':2,'id':'test','method':'download_video','params':{
        'input':'https://www.tiktok.com/@u/video/123','profiles':{'tiktok':{'proxy':proxy}},
        'expected':{'platform':'tiktok','video_id':'123'},'staging_dir':str(stage)}}
    process.stdin.write((json.dumps(request)+'\n').encode()); process.stdin.flush()
    try:
        assert started.wait(5)
        process.stdin.close()
        assert process.wait(timeout=5) == 3
        sizes = [(p.name,p.stat().st_size) for p in stage.iterdir()]
        assert b'download_finished' not in process.stdout.read()
        assert [(p.name,p.stat().st_size) for p in stage.iterdir()] == sizes
    finally:
        if process.poll() is None: process.kill(); process.wait()


def test_startup_timeout_kills_engine(tmp_path):
    script=tmp_path/'stalled.py'
    script.write_text('import time\ntime.sleep(60)\n')
    result=subprocess.run([str(PROBE),sys.executable,str(script)],
        input=json.dumps(config(tmp_path,None)).encode(),capture_output=True,timeout=20)
    assert result.returncode == 1
    assert b'TIMEOUT' in result.stdout
    assert b'completed' not in result.stdout


def test_parent_exit_ends_sidecar(tmp_path,media_proxy):
    proxy, _, started, _ = media_proxy
    process=subprocess.Popen([str(PROBE),sys.executable,str(FIXTURE)],stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,stderr=subprocess.PIPE,env={**os.environ,'FIXTURE_MODE':'stall',
            'FIXTURE_PID_FILE':str(tmp_path/'child.pid')})
    process.stdin.write(json.dumps(config(tmp_path,proxy)).encode()); process.stdin.close()
    try:
        assert started.wait(5)
        pid = int((tmp_path/'child.pid').read_text())
        if os.name == 'nt':
            import ctypes
            from ctypes import wintypes
            kernel = ctypes.WinDLL('kernel32', use_last_error=True)
            kernel.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
            kernel.OpenProcess.restype = wintypes.HANDLE
            kernel.WaitForSingleObject.argtypes = [wintypes.HANDLE, wintypes.DWORD]
            kernel.CloseHandle.argtypes = [wintypes.HANDLE]
            handle = kernel.OpenProcess(0x100000, False, pid)
            assert handle
        # The engine inherits only its stdin pipe from the probe. Abruptly killing
        # the probe closes that pipe and its EOF watcher interrupts the network.
        process.kill(); process.wait(timeout=5)
        if os.name == 'nt':
            try:
                assert kernel.WaitForSingleObject(handle, 5000) == 0
            finally:
                kernel.CloseHandle(handle)
        else:
            import time
            deadline = time.monotonic() + 5
            while time.monotonic() < deadline:
                try: os.kill(pid, 0)
                except ProcessLookupError: break
                time.sleep(.05)
            else: pytest.fail('sidecar remained alive after parent exit')
        process.stdout.read(); process.stderr.read()
    finally:
        if process.poll() is None: process.kill(); process.wait()


def test_cancel_probe_reaps_child_and_cleans_stage(tmp_path,media_proxy):
    import signal
    proxy, _, started, _ = media_proxy
    process=subprocess.Popen([str(PROBE),sys.executable,str(FIXTURE)],stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,stderr=subprocess.PIPE,env={**os.environ,'FIXTURE_MODE':'stall'},
        creationflags=subprocess.CREATE_NEW_PROCESS_GROUP if os.name == 'nt' else 0)
    process.stdin.write(json.dumps(config(tmp_path,proxy)).encode()); process.stdin.close()
    try:
        assert started.wait(5)
        process.send_signal(signal.CTRL_BREAK_EVENT if os.name == 'nt' else signal.SIGINT)
        assert process.wait(timeout=5) == 1
        output=process.stdout.read()
        assert b'CANCELED' in output
        assert b'completed' not in output
        assert list(tmp_path.iterdir()) == []
    finally:
        if process.poll() is None: process.kill(); process.wait()
