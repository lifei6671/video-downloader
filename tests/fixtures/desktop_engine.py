"""Offline protocol fixture for Rust desktop lifecycle tests, never production."""
import json
import hashlib
import os
import sys
import time
from pathlib import Path
from urllib.parse import parse_qs, urlsplit

dtk = json.loads((Path(__file__).resolve().parents[2] / "vendor/dtk/manifest.json").read_text())
print(json.dumps(dict(protocol_version=3, type="ready", engine_version="0.13.0", yt_dlp_version="2026.08.19", dtk_version=dtk["upstream_version"], dtk_commit=dtk["upstream_commit"])), flush=True)
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
if request['method'] == 'transcribe_video':
    assert set(params) == {'source', 'staging_dir', 'runtime_directory'}
    stage = Path(params['staging_dir'])
    mode = Path(params['source']).stem
    backend = 'cuda' if mode.startswith('cuda') else 'cpu'
    if backend == 'cuda': emit('transcription_progress', phase='segmenting', duration_seconds=3, backend=backend)
    emit('transcription_progress', phase='recognizing', duration_seconds=3, backend=backend)
    if mode.startswith('hold'):
        time.sleep(120)
    if mode.startswith('fail'):
        emit('error', error={'code':'TRANSCRIPTION_FAILED'}); sys.exit(1)
    text = '' if mode.startswith('silent') else '欢迎体验。\nEnglish text.'
    (stage / 'transcript.txt').write_text(text, encoding='utf-8', newline='\n')
    emit('transcribed', filename='transcript.txt', bytes=len(text.encode()), no_speech=not text, duration_seconds=3, backend=backend)
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
