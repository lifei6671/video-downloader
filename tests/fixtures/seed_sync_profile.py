"""Seed only an idle, marked WB305 GUI profile, using synthetic / prior test media."""
import hashlib
import json
import sqlite3
import uuid
from pathlib import Path


def prepare(root: Path) -> dict:
    meta = json.loads((root / '.local/p1a-acceptance/wb305-final/upgrade/profile.json').read_text(encoding='utf-8-sig'))
    data = Path(meta['data'])
    marker = json.loads((data / 'p1a-acceptance.json').read_text(encoding='utf-8'))
    assert marker['identifier'] == 'me.disign.yingqu.p1a-wb305-final-upgrade'
    assert Path(marker['workspace']).resolve() == root.resolve()
    db = sqlite3.connect(data / 'yingqu.sqlite')
    db.execute('PRAGMA foreign_keys=ON')
    assert db.execute('SELECT version FROM library_schema').fetchone()[0] in (6, 7, 8, 9, 10, 11)
    assert not db.execute("SELECT 1 FROM asset_documents WHERE model_name='WB305 fixture'").fetchone(), 'Already seeded'
    owner = db.execute("SELECT id FROM library_assets WHERE video_id='7300000000000000001'").fetchone()[0]
    original = next(row[0] for row in db.execute('SELECT id,path FROM asset_files WHERE asset_id=?', (owner,)) if row[1].endswith('原视频.mp4'))
    output = []
    def canonical(path: Path) -> str:
        value = str(path.resolve())
        return value if value.startswith('\\\\?\\') else '\\\\?\\' + value

    def document(asset_id: str, file_id: str, segments: list, frames: int) -> str:
        file = db.execute('SELECT path FROM asset_files WHERE id=?', (file_id,)).fetchone()
        path = Path(file[0]); stat = path.stat()
        source = dict(path=canonical(path), size=stat.st_size, mtime_ns=str(stat.st_mtime_ns))
        text = '\n'.join(segment['text'] for segment in segments)
        doc_id = str(uuid.uuid4())
        version = db.execute('SELECT coalesce(max(version),0)+1 FROM asset_documents WHERE asset_id=? AND kind=?', (asset_id, 'recognized')).fetchone()[0]
        db.execute('''INSERT INTO asset_documents(id,asset_id,file_id,kind,text,version,revision,source_json,model_name,model_revision,pipeline_revision,backend,created_at)
                      VALUES(?,?,?,'recognized',?,?,1,?,'WB305 fixture','wb305-fixture','wb305-fixture','cpu',1793000300)''', (doc_id, asset_id, file_id, text, version, json.dumps(source, ensure_ascii=False)))
        db.execute('INSERT INTO transcript_timings VALUES(?,?,?,?)', (doc_id, frames, '6840bae4c5c92ee8c04faaf4db23dd0105098d7f', 'wb305-' + doc_id))
        db.executemany('INSERT INTO transcript_segments VALUES(?,?,?,?,?)', [(doc_id, i, row['start_ms'], row['end_ms'], row['text']) for i, row in enumerate(segments)])
        output.append(dict(asset_id=asset_id, file_id=file_id, document_id=doc_id, version=version, segments=segments))
        return doc_id

    with db:
        document(owner, original, [dict(start_ms=200, end_ms=500, text='原生第一段😀'), dict(start_ms=800, end_ms=1200, text='原生第二段学习')], 24000)
        natural = root / '.local/wb305/natural-sync.mp4'
        asset_id, file_id = str(uuid.uuid4()), str(uuid.uuid4()); stat = natural.stat()
        db.execute('INSERT INTO library_assets VALUES(?,?,NULL,NULL,NULL,NULL,1793000400,1793000400,NULL)', (asset_id, 'WB305 实测语音段长样本'))
        db.execute('INSERT INTO asset_files VALUES(?,?,?,?,?,?,?,?)', (file_id, asset_id, canonical(natural), stat.st_size, str(stat.st_mtime_ns), 297586, 160, 90))
        prior = json.loads((root / '.local/wb301/run-complete/natural-zh/result.json').read_text(encoding='utf-8'))
        document(asset_id, file_id, prior['cuda_cues'], 4761383)
    db.close()
    report = dict(status='PREPARED', native_gui=False, cases=output, long_input_sha256=hashlib.sha256(natural.read_bytes()).hexdigest(), long_timing_source='WB301 actual CUDA VAD cues; prior test WAV muxed with synthetic gray video, no ASR rerun')
    (root / '.local/wb305/native-fixture.json').write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding='utf-8')
    return dict(status='PREPARED', cases=len(output), long_segments=len(prior['cuda_cues']))


if __name__ == '__main__':
    print(json.dumps(prepare(Path(__file__).resolve().parents[2])))
