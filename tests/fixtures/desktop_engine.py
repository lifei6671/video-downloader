"""Offline protocol fixture for Rust desktop lifecycle tests, never production."""
import json
import os
import sys
import time
from pathlib import Path

print(json.dumps(dict(protocol_version=2, type="ready", engine_version="0.2.1", yt_dlp_version="2026.08.19")), flush=True)
request = json.loads(sys.stdin.readline())
params = request["params"]
def emit(kind, **data):
    print(json.dumps(dict(protocol_version=2, id=request["id"], type=kind, **data)), flush=True)
if request["method"] == "resolve_video":
    if params["input"] == "invalid":
        emit("error", error={"code": "INVALID_INPUT"})
    elif params["input"] == "block":
        time.sleep(120)
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
    if video_id.startswith(("8", "7")):
        root = stage.parent.parent
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
