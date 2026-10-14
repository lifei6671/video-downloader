"""Offline protocol fixture for Rust desktop lifecycle tests, never production."""
import json
import hashlib
import os
import sys
import time
from pathlib import Path

dtk = json.loads((Path(__file__).resolve().parents[2] / "vendor/dtk/manifest.json").read_text())
print(json.dumps(dict(protocol_version=3, type="ready", engine_version="0.3.0", yt_dlp_version="2026.08.19", dtk_version=dtk["upstream_version"], dtk_commit=dtk["upstream_commit"])), flush=True)
request = json.loads(sys.stdin.readline())
params = request["params"]
def emit(event_type, **data):
    print(json.dumps(dict(protocol_version=3, id=request["id"], type=event_type, **data)), flush=True)
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
        emit("resolved", result=dict(platform="douyin", video_id=video_id,
            canonical_url="https://www.douyin.com/video/" + video_id,
            title="离线生命周期验证", author="fixture", duration_ms=1000))
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
        if video_id == "456":
            image = b"\x89PNG\r\n\x1a\nfixture"
            (stage / "cover").write_bytes(image)
            cover = dict(cover_filename="cover", cover_bytes=len(image))
        emit("download_finished", platform="douyin", video_id=video_id,
            filename="video.mp4", container="mp4", bytes=len(body), **cover)
