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
DTK = json.loads((ROOT / 'vendor/dtk/manifest.json').read_text())


def config(tmp_path, proxy):
    return {'inputs':['https://www.tiktok.com/@u/video/123'],
            'profiles':{'tiktok':{'proxy':proxy,'cookie':'session=must-not-leak'}},'output_dir':str(tmp_path)}


@pytest.mark.parametrize('mode', ['estimate', 'unknown', 'no_progress', 'bad_eta', 'bad_speed',
    'bad_count', 'zero_total', 'bad_estimate', 'exact_mismatch', 'regression'])
def test_bridge_progress_optional_fields_and_completion_validation(tmp_path, mode):
    script = tmp_path / 'progress.py'
    script.write_text('''import json,sys
from pathlib import Path
dtk=json.loads(Path(sys.argv[1]).read_text())
mode=sys.argv[2]
print(json.dumps(dict(type='ready',protocol_version=3,engine_version='0.13.0',yt_dlp_version='2026.08.19',dtk_version=dtk['upstream_version'],dtk_commit=dtk['upstream_commit'])),flush=True)
r=json.loads(sys.stdin.readline())
def emit(kind,**data): print(json.dumps(dict(type=kind,protocol_version=3,id=r['id'],**data)),flush=True)
if r['method']=='resolve_video':
 emit('resolved',result=dict(platform='tiktok',video_id='123',canonical_url='https://www.tiktok.com/@u/video/123'))
else:
 body=b'\\x00\\x00\\x00\\x18ftypmp42fixture'
 (Path(r['params']['staging_dir'])/'video.mp4').write_bytes(body)
 data=dict(downloaded_bytes=len(body),total_bytes=None,total_bytes_estimate=999 if mode=='estimate' else None,speed_bps=0,eta_seconds=2.5)
 if mode=='bad_eta':data['eta_seconds']=-1
 if mode=='bad_speed':data['speed_bps']=1e20
 if mode=='bad_count':data['downloaded_bytes']=-1
 if mode=='zero_total':data['total_bytes']=0
 if mode=='bad_estimate':data['total_bytes_estimate']=-1
 if mode=='exact_mismatch':data['total_bytes']=len(body)+1
 if mode!='no_progress':emit('progress',**data)
 if mode=='regression':emit('progress',**{**data,'downloaded_bytes':0})
 emit('download_finished',platform='tiktok',video_id='123',filename='video.mp4',container='mp4',bytes=len(body))
''', encoding='utf-8')
    result = subprocess.run([str(PROBE), sys.executable, str(script), str(ROOT / 'vendor/dtk/manifest.json'), mode],
        input=json.dumps(config(tmp_path, None)).encode(), capture_output=True, timeout=15)
    success = mode in ('estimate', 'unknown')
    assert result.returncode == (0 if success else 1), (result.stdout, result.stderr)
    if success:
        events = [json.loads(line) for line in result.stdout.splitlines()]
        progress = next(e for e in events if e.get('event') == 'progress')
        assert progress['eta_seconds'] == 2.5 and progress['speed_bps'] == 0
        assert progress['total_bytes_estimate'] == (999 if mode == 'estimate' else None)
        assert progress['total_bytes'] is None
    else:
        assert b'"event":"completed"' not in result.stdout
        assert not (tmp_path / 'tiktok_123.mp4').exists()
    assert not any(path.name.startswith('yingqu-') for path in tmp_path.iterdir())


@pytest.mark.parametrize('bad_transfer', [False, True])
def test_bridge_merged_file_size_is_distinct_from_transferred_bytes(tmp_path, bad_transfer):
    script = tmp_path / 'merged.py'
    script.write_text('''import json,sys
from pathlib import Path
dtk=json.loads(Path(sys.argv[1]).read_text())
print(json.dumps(dict(type='ready',protocol_version=3,engine_version='0.13.0',yt_dlp_version='2026.08.19',dtk_version=dtk['upstream_version'],dtk_commit=dtk['upstream_commit'])),flush=True)
r=json.loads(sys.stdin.readline())
def emit(kind,**data): print(json.dumps(dict(type=kind,protocol_version=3,id=r['id'],**data)),flush=True)
if r['method']=='resolve_video':
 emit('resolved',result=dict(platform='youtube',video_id='BaW_jenozKc',canonical_url='https://www.youtube.com/watch?v=BaW_jenozKc'))
else:
 body=b'\\x00\\x00\\x00\\x18ftypmp42merged'
 (Path(r['params']['staging_dir'])/'video.mp4').write_bytes(body)
 emit('progress',downloaded_bytes=100,total_bytes=None,speed_bps=None)
 emit('progress',downloaded_bytes=100,total_bytes=None,speed_bps=None,phase='merging')
 emit('download_finished',platform='youtube',video_id='BaW_jenozKc',filename='video.mp4',container='mp4',bytes=len(body),transferred_bytes=99 if sys.argv[2]=='bad' else 100)
''', encoding='utf-8')
    inputs = {'inputs': ['https://www.youtube.com/watch?v=BaW_jenozKc'], 'profiles': {}, 'output_dir': str(tmp_path)}
    result = subprocess.run([str(PROBE), sys.executable, str(script), str(ROOT / 'vendor/dtk/manifest.json'),
                             'bad' if bad_transfer else 'good'],
        input=json.dumps(inputs).encode(), capture_output=True, timeout=15)
    assert result.returncode == (1 if bad_transfer else 0), (result.stdout, result.stderr)
    assert (tmp_path / 'youtube_BaW_jenozKc.mp4').exists() is (not bad_transfer)
    if bad_transfer:
        assert b'INCOMPLETE_MEDIA' in result.stdout


def test_protocol_error_reaps_engine_descendant(tmp_path):
    script = tmp_path / 'descendant.py'
    pid_file = tmp_path / 'descendant.pid'
    script.write_text('''import json,sys,subprocess
from pathlib import Path
dtk=json.loads(Path(sys.argv[1]).read_text())
print(json.dumps(dict(type='ready',protocol_version=3,engine_version='0.13.0',yt_dlp_version='2026.08.19',dtk_version=dtk['upstream_version'],dtk_commit=dtk['upstream_commit'])),flush=True)
r=json.loads(sys.stdin.readline())
child=subprocess.Popen([sys.executable,'-c','import time;time.sleep(60)'],stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
Path(sys.argv[2]).write_text(str(child.pid))
print(json.dumps(dict(type='resolved',protocol_version=3,id='wrong')),flush=True)
sys.stdin.read()
''', encoding='utf-8')
    result = subprocess.run([str(PROBE), sys.executable, str(script), str(ROOT / 'vendor/dtk/manifest.json'), str(pid_file)],
        input=json.dumps(config(tmp_path, None)).encode(), capture_output=True, timeout=15)
    assert result.returncode == 1 and b'ENGINE_PROTOCOL_ERROR' in result.stdout
    pid = int(pid_file.read_text())
    if os.name == 'nt':
        import ctypes
        from ctypes import wintypes
        kernel = ctypes.WinDLL('kernel32', use_last_error=True)
        kernel.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
        kernel.OpenProcess.restype = wintypes.HANDLE
        kernel.WaitForSingleObject.argtypes = [wintypes.HANDLE, wintypes.DWORD]
        kernel.CloseHandle.argtypes = [wintypes.HANDLE]
        handle = kernel.OpenProcess(0x100000, False, pid)
        if handle:
            try:
                assert kernel.WaitForSingleObject(handle, 5000) == 0
            finally:
                kernel.CloseHandle(handle)
    else:
        # A killed descendant can briefly remain a zombie until init reaps it.
        import time
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            status = subprocess.run(['ps', '-o', 'stat=', '-p', str(pid)], capture_output=True, text=True).stdout.strip()
            if not status or status.startswith('Z'):
                break
            time.sleep(.05)
        else:
            pytest.fail('external runtime descendant survived bridge cleanup')


@pytest.mark.skipif(os.name != 'nt', reason='Windows exclusive media-handle cleanup contract')
def test_cancel_waits_until_descendant_media_handle_is_released(tmp_path):
    import signal
    script = tmp_path / 'holding.py'
    marker = tmp_path / 'holding.pid'
    script.write_text('''import json,sys,subprocess,time
from pathlib import Path
dtk=json.loads(Path(sys.argv[1]).read_text())
print(json.dumps(dict(type='ready',protocol_version=3,engine_version='0.13.0',yt_dlp_version='2026.08.19',dtk_version=dtk['upstream_version'],dtk_commit=dtk['upstream_commit'])),flush=True)
r=json.loads(sys.stdin.readline())
def emit(kind,**data): print(json.dumps(dict(type=kind,protocol_version=3,id=r['id'],**data)),flush=True)
if r['method']=='resolve_video':
 emit('resolved',result=dict(platform='youtube',video_id='BaW_jenozKc',canonical_url='https://www.youtube.com/watch?v=BaW_jenozKc'))
else:
 code="""import ctypes,sys,time,os
from ctypes import wintypes
from pathlib import Path
k=ctypes.WinDLL('kernel32',use_last_error=True)
k.CreateFileW.argtypes=[wintypes.LPCWSTR,wintypes.DWORD,wintypes.DWORD,wintypes.LPVOID,wintypes.DWORD,wintypes.DWORD,wintypes.HANDLE]
k.CreateFileW.restype=wintypes.HANDLE
h=k.CreateFileW(sys.argv[1],0x40000000,0,None,2,0,None)
assert h not in (None,ctypes.c_void_p(-1).value)
Path(sys.argv[2]).write_text(str(os.getpid()))
time.sleep(60)
"""
 subprocess.Popen([sys.executable,'-c',code,str(Path(r['params']['staging_dir'])/'locked.part'),sys.argv[2]],stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
 deadline=time.monotonic()+5
 while not Path(sys.argv[2]).exists():
  assert time.monotonic()<deadline
  time.sleep(.01)
 emit('progress',downloaded_bytes=1,total_bytes=None,speed_bps=None)
 sys.stdin.read()
''', encoding='utf-8')
    process = subprocess.Popen([str(PROBE), sys.executable, str(script), str(ROOT / 'vendor/dtk/manifest.json'), str(marker)],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        creationflags=subprocess.CREATE_NEW_PROCESS_GROUP)
    inputs = {'inputs': ['https://www.youtube.com/watch?v=BaW_jenozKc'], 'profiles': {}, 'output_dir': str(tmp_path)}
    process.stdin.write(json.dumps(inputs).encode())
    process.stdin.close()
    try:
        assert json.loads(process.stdout.readline())['event'] == 'resolved'
        assert json.loads(process.stdout.readline())['event'] == 'progress'
        process.send_signal(signal.CTRL_BREAK_EVENT)
        assert process.wait(timeout=5) == 1
        output = process.stdout.read()
        assert b'CANCELED' in output and b'IO_ERROR' not in output
        assert not any(p.name.startswith('yingqu-') for p in tmp_path.iterdir())
    finally:
        if process.poll() is None:
            process.kill()
            process.wait()


@pytest.mark.parametrize('mode', ['good','unknown'])
def test_actual_downloader_to_rust_finalize(mode, tmp_path, media_proxy):
    proxy, requests, _, _ = media_proxy
    env = {**os.environ,'FIXTURE_MODE':mode}
    run = lambda: subprocess.run([str(PROBE),sys.executable,str(FIXTURE)],
        input=json.dumps(config(tmp_path,proxy)).encode(),capture_output=True,env=env,timeout=15)
    result = run()
    assert result.returncode == 0, (result.stdout,result.stderr)
    assert b'"event":"completed"' in result.stdout and b'must-not-leak' not in result.stdout + result.stderr
    assert (tmp_path/'tiktok_123.mp4').read_bytes() == BODY
    assert not any('Cookie' in headers for _,headers in requests)
    assert list(tmp_path.iterdir()) == [tmp_path/'tiktok_123.mp4']
    second = run()
    assert second.returncode == 1
    assert b'"event":"completed"' not in second.stdout
    assert (tmp_path/'tiktok_123.mp4').read_bytes() == BODY


@pytest.mark.parametrize('mode',['html','truncated','redirect','segmented','playlist'])
def test_rejects_bad_media_and_cleans_staging(mode,tmp_path,media_proxy):
    proxy, requests, _, _ = media_proxy
    result=subprocess.run([str(PROBE),sys.executable,str(FIXTURE)],
        input=json.dumps(config(tmp_path,proxy)).encode(),capture_output=True,
        env={**os.environ,'FIXTURE_MODE':mode},timeout=15)
    assert result.returncode == 1
    assert b'"event":"completed"' not in result.stdout
    assert list(tmp_path.iterdir()) == []
    if mode == 'redirect':
        assert len(requests) == 1  # malicious redirect never reaches the proxy


@pytest.mark.parametrize('mode',['bad_ready','wrong_version','wrong_id','eof','duplicate','nonzero','secret_error'])
def test_bridge_stops_bad_engine(mode,tmp_path):
    script=tmp_path/'fake.py'
    script.write_text('''import json,sys
mode=sys.argv[1]
print(json.dumps({'type':'ready','protocol_version':1 if mode=='wrong_version' else 3,
 'engine_version':'bad' if mode=='bad_ready' else '0.13.0','yt_dlp_version':'2026.08.19','dtk_version':sys.argv[2],'dtk_commit':sys.argv[3]}),flush=True)
request=json.loads(sys.stdin.readline())
if mode=='eof': sys.exit(0)
if mode=='secret_error':
 print(json.dumps({'protocol_version':3,'id':request['id'],'type':'error','error':{'code':'session=private','message':'secret'}}),flush=True)
 sys.exit(1)
msg={'protocol_version':3,'id':'wrong' if mode=='wrong_id' else request['id'],'type':'resolved','result':{'platform':'tiktok','video_id':'123','canonical_url':'https://www.tiktok.com/@u/video/123'}}
print(json.dumps(msg),flush=True)
if mode=='duplicate': print(json.dumps(msg),flush=True)
if mode=='nonzero': sys.exit(2)
if mode=='wrong_id': sys.stdin.read()
''')
    result=subprocess.run([str(PROBE),sys.executable,str(script),mode,DTK['upstream_version'],DTK['upstream_commit']],
        input=json.dumps(config(tmp_path,None)).encode(),capture_output=True,timeout=20)
    assert result.returncode == 1
    assert b'ENGINE_PROTOCOL_ERROR' in result.stdout
    assert b'"event":"completed"' not in result.stdout
    assert b'private' not in result.stdout + result.stderr


@pytest.mark.parametrize('chunk', ['0', '10485760'])
def test_eof_cancels_blocked_network(tmp_path,media_proxy,chunk):
    proxy, _, started, _ = media_proxy
    stage=tmp_path/'stage'; stage.mkdir()
    process=subprocess.Popen([sys.executable,str(FIXTURE)],stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,stderr=subprocess.PIPE,env={**os.environ,'FIXTURE_MODE':'stall', 'YINGQU_HTTP_CHUNK_SIZE':chunk})
    assert json.loads(process.stdout.readline())['type']=='ready'
    request={'protocol_version':3,'id':'test','method':'download_video','params':{
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
    assert b'"event":"completed"' not in result.stdout


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
        assert b'"event":"completed"' not in output
        assert list(tmp_path.iterdir()) == []
    finally:
        if process.poll() is None: process.kill(); process.wait()
