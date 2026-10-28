"""Synthetic Windows GUI profiles. Never reads user settings or credentials."""
import argparse
import json
import os
import shutil
import sqlite3
from pathlib import Path


def prepare(data: Path, output: Path, scenario: str) -> dict:
    data.mkdir(parents=True, exist_ok=True)
    output.mkdir(parents=True, exist_ok=True)
    media = output / "media"
    media.mkdir(exist_ok=True)
    fixture = Path(__file__).parent / "media" / "player-portrait.mp4"
    for name in ["原视频.mp4", "另一个版本.mp4", "待导入.mp4"]:
        if not (media / name).exists():
            shutil.copyfile(fixture, media / name)
    db_path = data / "yingqu.sqlite"
    if db_path.exists():
        return {"database": str(db_path), "existing": True}
    db = sqlite3.connect(db_path)
    db.execute("CREATE TABLE app_settings(id INTEGER PRIMARY KEY, directory TEXT NOT NULL)")
    db.execute("INSERT INTO app_settings VALUES(1,?)", (str(media.resolve()),))
    if scenario == "upgrade":
        db.execute("CREATE TABLE download_tasks(id TEXT PRIMARY KEY,snapshot TEXT NOT NULL)")
        db.execute("CREATE TABLE video_transcripts(task_id TEXT PRIMARY KEY,snapshot TEXT NOT NULL)")
        for i, (video_id, filename, kind) in enumerate([
            ("7300000000000000001", "原视频.mp4", "video"),
            ("7300000000000000001", "原视频.mp4", "video"),
            ("7300000000000000001", "另一个版本.mp4", "video"),
            ("7300000000000000002", "缺失视频.mp4", "video"),
            ("7300000000000000003", "旧图集", "image_album"),
        ]):
            path = media / filename
            task_id = f"legacy-{i}"
            task = dict(id=task_id, revision=1, status="completed", video=dict(
                platform="douyin", video_id=video_id, kind=kind,
                resolver="dtk" if kind == "image_album" else "yt-dlp", title="旧视频学习资料" if kind == "video" else "旧图集",
                author="合成作者", canonical_url=f"https://www.douyin.com/video/{video_id}"
            ), bytes=path.stat().st_size if path.is_file() else 0, total_bytes=None,
                speed_bps=None, error=None, path=str(path.resolve()), created_at=100+i,
                completed_at=200+i, canceling=False, download_requested=True, files=[])
            db.execute("INSERT INTO download_tasks VALUES(?,?)", (task_id, json.dumps(task, ensure_ascii=False)))
            if i in [0, 3]:
                source_path = str(path.resolve())
                if os.name == "nt":
                    # Source::read stores std::fs::canonicalize's verbatim Windows path.
                    source_path = "\\\\?\\" + source_path
                source = dict(path=source_path, size=path.stat().st_size if path.is_file() else 123,
                              mtime_ns=str(path.stat().st_mtime_ns) if path.is_file() else "1")
                result = dict(text=f"中文迁移原文 {i}，学习开场结构与表达节奏。", source=source,
                              model_revision="synthetic-legacy", model_name="旧模型", pipeline_revision="fixture", backend="cpu", completed_at=300+i)
                snapshot = dict(task_id=task_id, operation_id=f"old-{i}", sequence=1, state="completed", phase=None,
                                error_code=None, updated_at=300+i, result=result, has_result=True, source_changed=False)
                db.execute("INSERT INTO video_transcripts VALUES(?,?)", (task_id, json.dumps(snapshot, ensure_ascii=False)))
    db.commit()
    db.close()
    return {"database": str(db_path), "existing": False}


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--data", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--scenario", choices=["empty", "upgrade", "restore"], required=True)
    args = parser.parse_args()
    print(json.dumps(prepare(args.data, args.output, args.scenario), ensure_ascii=False))
