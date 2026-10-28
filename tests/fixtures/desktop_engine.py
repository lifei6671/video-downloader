"""Offline protocol fixture for Rust desktop lifecycle tests, never production."""
import json
import hashlib
import os
import sys
import time
from pathlib import Path
from urllib.parse import parse_qs, urlsplit

sys.stdin.reconfigure(encoding="utf-8")
sys.stdout.reconfigure(encoding="utf-8")

dtk = json.loads((Path(__file__).resolve().parents[2] / "vendor/dtk/manifest.json").read_text())
print(json.dumps(dict(protocol_version=3, type="ready", engine_version="0.17.0", yt_dlp_version="2026.08.19", dtk_version=dtk["upstream_version"], dtk_commit=dtk["upstream_commit"])), flush=True)
request = json.loads(sys.stdin.readline())
params = request["params"]
source = urlsplit(params.get('input', ''))
platform = 'youtube' if source.hostname == 'www.youtube.com' else 'tiktok' if source.hostname == 'www.tiktok.com' else 'douyin'
def emit(event_type, **data):
    print(json.dumps(dict(protocol_version=3, id=request["id"], type=event_type, **data)), flush=True)
if request['method'] in ('check_dependencies', 'install_dependency'):
    assert set(params).issubset({'runtime_directory', 'tool', 'staging_dir', 'variant'})
    assert Path(params['runtime_directory']).is_absolute()
    if request['method'] == 'install_dependency':
        assert Path(params['staging_dir']).parent == Path(params['runtime_directory'])
        (Path(params['staging_dir']) / 'partial').write_bytes(b'partial download')
        emit('dependency_progress', tool=params['tool'], phase='downloading', downloaded_bytes=10, total_bytes=20)
        if params['tool'] == 'javascript':
            time.sleep(120)
        emit('dependency_progress', tool=params['tool'], phase='installing', downloaded_bytes=20, total_bytes=20)
        mode_file = Path(params['runtime_directory']).parent / 'dependency-commit-mode'
        if params['tool'] == 'fun-asr-nano' and mode_file.is_file():
            stage, root = Path(params['staging_dir']), Path(params['runtime_directory'])
            name = 'fun-asr-nano-cuda' if params.get('variant') == 'cuda' else 'fun-asr-nano'
            prepared = stage / 'prepared'; prepared.mkdir()
            (prepared / 'new-model').write_bytes(b'new')
            previous = stage / 'previous'; previous.mkdir()
            if (root / name).exists(): (root / name).rename(previous / name)
            if mode_file.read_text() == 'between-renames':
                mode_file.with_name('dependency-commit-started').write_text('ready')
                time.sleep(120)
            prepared.rename(root / name)
            (root / 'fun-asr-nano-backend.json').write_text(json.dumps(dict(variant=params.get('variant'))))
            mode_file.with_name('dependency-commit-started').write_text('ready')
            time.sleep(120)
    emit('dependencies', result=dict(platform='fixture', directory=params['runtime_directory'], tools=[]))
    sys.exit(0)
if request['method'] == 'extract_keyframes':
    import base64
    import subprocess
    stage = Path(params['staging_dir'])
    mode = Path(params['source']).stem
    png = base64.b64decode('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGNgYGAAAAAEAAH2FzhVAAAAAElFTkSuQmCC')
    if mode.startswith('hold'):
        sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'python-engine'))
        from local_tools import installed_tool
        ffmpeg = installed_tool('ffmpeg')
        command = ([ffmpeg, '-nostdin', '-v', 'error', '-re', '-f', 'lavfi', '-i', 'color=c=black:s=16x16:r=1', '-t', '120', '-f', 'null', '-'] if ffmpeg else [sys.executable, '-c', 'import time;time.sleep(120)'])
        child = subprocess.Popen(command, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, creationflags=0x08000000 if sys.platform == 'win32' else 0)
        (stage / 'partial').write_bytes(b'partial')
        (Path(params['runtime_directory']).parent / 'keyframes-started.json').write_text(json.dumps(dict(engine=os.getpid(), child=child.pid, ffmpeg=bool(ffmpeg), stage=str(stage))))
        time.sleep(120)
    if mode.startswith('changed'): Path(params['source']).write_bytes(b'changed source')
    frames=[]
    for i, position in enumerate(params['positions_ms']):
        name=f'frame-{i:03}.png';(stage/name).write_bytes(png)
        frames.append(dict(filename=name,requested_ms=position,pts=position,time_base_num=1,time_base_den=1000,width=1,height=1,black=False,bytes=len(png)))
        emit('keyframe_progress', completed=i+1, total=len(params['positions_ms']))
    if mode.startswith('bad-frame'): frames[-1]['time_base_den']=0
    if mode.startswith('fail-frame'): emit('error',error=dict(code='KEYFRAME_FAILED'));sys.exit(1)
    emit('keyframes_extracted',frames=frames);sys.exit(0)
if request['method'] == 'probe_local_video':
    assert set(params) == {'source', 'staging_dir', 'runtime_directory'}
    mode = Path(params['source']).stem
    if mode.startswith('hold'):
        (Path(params['runtime_directory']).parent / 'probe-started').write_text(str(os.getpid()))
        time.sleep(120)
    if mode.startswith('invalid'):
        emit('error', error={'code': 'LOCAL_VIDEO_INVALID'}); sys.exit(1)
    if mode.startswith('changed'):
        Path(params['source']).write_bytes(b'changed file')
    emit('local_video_probed', width=1280, height=720, duration_ms=3000)
    sys.exit(0)
if request['method'] == 'transcribe_video':
    assert set(params) == {'source', 'staging_dir', 'runtime_directory'}
    stage = Path(params['staging_dir'])
    mode = Path(params['source']).stem
    backend = 'cuda' if mode.startswith('cuda') else 'cpu'
    if backend == 'cuda': emit('transcription_progress', phase='segmenting', duration_seconds=3, backend=backend)
    emit('transcription_progress', phase='loading_model', duration_seconds=3, backend=backend)
    emit('transcription_progress', phase='recognizing', duration_seconds=3, backend=backend, completed_segments=1, total_segments=2)
    if mode.startswith('bad-progress-'):
        bad = {'backwards': (0, 2), 'total': (1, 3), 'overflow': (3, 2), 'missing': (None, 2), 'type': (True, 2)}[mode.removeprefix('bad-progress-')]
        emit('transcription_progress', phase='recognizing', duration_seconds=3, backend=backend,
             completed_segments=bad[0], total_segments=bad[1])
    if mode.startswith('hold'):
        if mode.startswith('hold-mcp'):
            import subprocess
            child = subprocess.Popen([sys.executable, '-c', 'import time;time.sleep(120)'], stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, creationflags=0x08000000 if sys.platform == 'win32' else 0)
            (stage / 'partial').write_bytes(b'partial')
            marker = Path(params['runtime_directory']).parent / 'mcp-transcription-started.json'
            temporary = marker.with_suffix('.tmp')
            temporary.write_text(json.dumps(dict(engine=os.getpid(), child=child.pid, stage=str(stage))), encoding='utf-8')
            temporary.replace(marker)
        time.sleep(120)
    if mode.startswith('fail'):
        emit('error', error={'code':'TRANSCRIPTION_FAILED'}); sys.exit(1)
    emit('transcription_progress', phase='recognizing', duration_seconds=3, backend=backend, completed_segments=2, total_segments=2)
    text = '' if mode.startswith('silent') else '欢迎体验。\nEnglish text.'
    (stage / 'transcript.txt').write_text(text, encoding='utf-8', newline='\n')
    timing = dict(schema_version=1, time_base='media_zero', timing_source='fsmn_vad', precision='vad_segment',
                  sample_rate=16000, frames=48000, vad_revision='6840bae4c5c92ee8c04faaf4db23dd0105098d7f',
                  segments=[] if not text else [dict(start_ms=0,end_ms=400,text='欢迎体验。'),dict(start_ms=600,end_ms=1000,text='English text.')])
    if mode.startswith('timing-'):
        flaw = mode.removeprefix('timing-')
        if flaw == 'overlap': timing['segments'][1]['start_ms'] = 300
        if flaw == 'version': timing['schema_version'] = 99
        if flaw == 'text': timing['segments'][0]['text'] = 'wrong'
        if flaw == 'duration': timing['frames'] = 1
    body = json.dumps(timing,ensure_ascii=False).encode()
    (stage / 'transcript-segments.json').write_bytes(body)
    descriptor = dict(filename='transcript-segments.json', bytes=len(body))
    if mode == 'timing-path': descriptor['filename'] = '../transcript-segments.json'
    if mode == 'timing-size': descriptor['bytes'] += 1
    emit('transcribed', filename='transcript.txt', bytes=len(text.encode()), no_speech=not text, duration_seconds=3, backend=backend, timing=descriptor)
    sys.exit(0)
if request['method'] == 'extract_cover':
    assert set(params) == {'runtime_directory', 'staging_dir', 'filename'}
    stage = Path(params['staging_dir'])
    assert (stage / params['filename']).is_file()
    root = stage.parent.parent.parent
    mode = (root / 'cover-mode').read_text() if (root / 'cover-mode').exists() else 'success'
    (root / 'cover-started').write_text(str(os.getpid()))
    if mode == 'block':
        time.sleep(120)
    if mode == 'failure':
        emit('cover_extracted', cover_error='COVER_UNAVAILABLE')
    else:
        body = b'\x89PNG\r\n\x1a\nfixture'
        (stage / 'cover').write_bytes(body)
        emit('cover_extracted', cover_filename='cover', cover_bytes=len(body))
    sys.exit(0)
if request["method"] == "identify_input":
    path=urlsplit(params["input"]).path
    album="/note/" in path
    video_id=path.rstrip("/").split("/")[-1]
    if platform == 'youtube': video_id=parse_qs(source.query)['v'][0]
    if path.startswith("/share"): video_id="601" if "album" in path else "123"
    canonical = params['input'] if platform != 'douyin' else f"https://www.douyin.com/{'note' if album or video_id=='601' else 'video'}/{video_id}"
    emit("identified",result=dict(platform=platform,video_id=video_id,canonical_url=canonical,kind="image_album" if album or video_id=='601' else "video",resolver="dtk" if album or video_id=='601' or params.get("resolver")=="dtk" else "yt-dlp"))
    sys.exit(0)
if request["method"] == "resolve_video":
    if params["input"] == "invalid":
        emit("error", error={"code": "INVALID_INPUT"})
    elif params["input"] == "block":
        time.sleep(120)
    elif params["input"] == "album" or "/note/601" in params["input"]:
        images=[]
        for index, ext in enumerate(("jpeg", "webp"), 1):
            body=b"\xff\xd8\xfffixture" if ext=="jpeg" else b"RIFF\x10\x00\x00\x00WEBPfixture"
            name=f"preview-{index:03d}.{ext}"
            if params.get("preview_dir"): (Path(params["preview_dir"])/name).write_bytes(body)
            images.append(dict(width=100,height=200,quality_source="high_definition",preview_filename=name))
        emit("resolved", result=dict(platform="douyin",video_id="601",canonical_url="https://www.douyin.com/note/601",kind="image_album",resolver="dtk",title="离线图集",images=images))
    else:
        video_id = params["input"].split(":", 1)[1] if params["input"].startswith("hold:") else "999" if params["input"] == "slow" else "123"
        if params["input"].startswith("https://www.douyin.com/video/"):
            video_id = params["input"].rsplit("/", 1)[1]
        elif platform == 'tiktok': video_id=source.path.rsplit('/',1)[1]
        elif platform == 'youtube': video_id=parse_qs(source.query)['v'][0]
        cover={}
        if params.get("preview_dir"):
            image=b"\x89PNG\r\n\x1a\nfixture"
            (Path(params["preview_dir"])/"cover").write_bytes(image)
            cover=dict(cover_filename="cover",cover_bytes=len(image))
            (Path(params['preview_dir'])/'author-avatar').write_bytes(image)
            cover.update(author_avatar_filename='author-avatar',author_avatar_bytes=len(image))
        if params["input"] == "cookie-warning":
            cover['cookie_warning'] = 'BROWSER_COOKIES_UNAVAILABLE'
        emit("resolved", result=dict(platform=platform, video_id=video_id,
            canonical_url=params['input'] if platform != 'douyin' else "https://www.douyin.com/video/" + video_id,
            title="离线生命周期验证", author="fixture", author_profile=dict(id='fixture-author',nickname='作者昵称',handle='fixture',kind='user'), duration_ms=1000, resolver=params.get("resolver") if params.get("resolver") in ("dtk","yt-dlp") else "yt-dlp",kind="video", width=1080, height=1920, selected_format_id="1"*24,
            formats=[dict(format_id="1"*24,container="mp4",width=1080,height=1920,filesize=40),
                     dict(format_id="2"*24,container="mp4",width=720,height=1280,filesize=20)], **cover))
else:
    stage = Path(params["staging_dir"])
    video_id = params["expected"]["video_id"]
    if params["expected"].get("kind") == "image_album":
        album=stage/"album"; album.mkdir()
        files=[];total=0
        for index, ext in enumerate(("jpeg", "webp"), 1):
            body=b"\xff\xd8\xfffixture" if ext=="jpeg" else b"RIFF\x10\x00\x00\x00WEBPfixture"
            name=f"{index:03d}.{ext}"; (album/name).write_bytes(body);total+=len(body)
            files.append(dict(filename=name,bytes=len(body),sha256=hashlib.sha256(body).hexdigest(),width=100,height=200,quality_source="high_definition"))
            emit("progress",downloaded_bytes=total,total_bytes=None,speed_bps=None,completed_items=index,total_items=2)
        emit("progress",downloaded_bytes=total,total_bytes=total,speed_bps=None,completed_items=2,total_items=2)
        emit("download_finished",platform="douyin",video_id=video_id,kind="image_album",resolver="dtk",files=files,bytes=total)
        sys.exit(0)
    if video_id.startswith(("8", "7")):
        root = stage.parent.parent.parent if stage.name == "media" else stage.parent.parent
        (root / ("started-" + video_id)).write_text(json.dumps(params["profiles"]))
        (stage / "video.mp4.part").write_bytes(b"partial")
        emit("progress", downloaded_bytes=7, total_bytes=None, speed_bps=None)
        while not (root / ("release-" + video_id)).exists():
            time.sleep(.01)
        if video_id.startswith("7"):
            emit("error", error={"code": "NETWORK_ERROR"})
            sys.exit(0)
        (stage / "video.mp4.part").unlink()
    if video_id == "999":
        with (stage / "video.mp4.part").open("wb") as out:
            for i in range(500):
                out.write(b"x" * 1024)
                out.flush()
                emit("progress", downloaded_bytes=(i + 1)*1024, total_bytes=None, speed_bps=None)
                time.sleep(.03)
    else:
        body = b"\x00\x00\x00\x18ftypmp42fixture"
        (stage / "video.mp4").write_bytes(body)
        emit("progress", downloaded_bytes=len(body), total_bytes=len(body), speed_bps=None)
        cover = {}
        if video_id in ("456", "fixture0001"):
            image = b"\x89PNG\r\n\x1a\ndownloaded-fixture"
            (stage / "cover").write_bytes(image)
            cover = dict(cover_filename="cover", cover_bytes=len(image))
        emit("download_finished", platform=params['expected']['platform'], video_id=video_id,
            filename="video.mp4", container="mp4", bytes=len(body), format_id=params.get("format_id"),
            **({'transferred_bytes':len(body)} if params['expected']['platform']=='youtube' else {}),
            width=720 if params.get("format_id")=="2"*24 else 1080,height=1280 if params.get("format_id")=="2"*24 else 1920, **cover)
