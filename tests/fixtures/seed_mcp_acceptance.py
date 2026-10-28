"""WB-208 synthetic data for an explicitly marked, empty native acceptance profile."""
import argparse
import hashlib
import json
import shutil
import sqlite3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
TEXT_A = '方法一😀：你是不是总把十分钟的任务拖成一整天？我把计时器设成十分钟，只允许自己先做第一步。第一个镜头是桌上的计时器特写，第二个镜头切到手写清单。每完成一步，就在清单上划掉一项。这个例子说明：把任务拆到能立刻动手，比反复催自己更容易开始。你可以先列出一件两分钟就能开始的小事。'
TEXT_B = '方法二：先看这两张清单：左边只写“做视频”，右边写“找一个例子、写三句话、录一段开头”。我没有使用倒计时，而是用左右对照展示动作差异。口播每讲到一个动作，画面就切到对应的勾选。大任务难开始，常常是因为下一步不够具体。试着把今天最模糊的一项任务改写成一个动作。'


def connect(data):
    marker = json.loads((data / 'wb208-acceptance.json').read_text(encoding='utf-8-sig'))
    if marker != {'purpose': 'wb208-synthetic', 'workspace': str(ROOT)}:
        raise ValueError('Refusing unmarked or unrelated acceptance profile')
    db = sqlite3.connect(f'{(data / "yingqu.sqlite").resolve().as_uri()}?mode=rw', uri=True)
    db.execute('PRAGMA foreign_keys=ON')
    if db.execute('SELECT version FROM library_schema WHERE id=1').fetchone() not in [(3,), (4,), (5,), (6,), (7,), (8,), (9,), (10,), (11,)]:
        db.close()
        raise ValueError('Initialize the isolated profile with the native app first')
    return db


def snapshot(data):
    db = connect(data)
    try:
        tables = [row[0] for row in db.execute("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")]
        result = {}
        for table in tables:
            # Table names come from SQLite metadata, never from MCP/user input.
            rows = db.execute('SELECT * FROM "' + table.replace('"', '""') + '"').fetchall()
            encoded = sorted(json.dumps(row, ensure_ascii=False, default=lambda value: {'blob_sha256': hashlib.sha256(value).hexdigest()}) for row in rows)
            result[table] = {'rows': len(rows), 'sha256': hashlib.sha256('\n'.join(encoded).encode()).hexdigest()}
        return result
    finally:
        db.close()


def seed(data, output):
    db = connect(data)
    try:
        if db.execute('SELECT count(*) FROM library_assets').fetchone()[0]:
            raise ValueError('Refusing to seed a nonempty library')
        media = output / 'media'
        media.mkdir(parents=True, exist_ok=True)
        texts = {'wb208-doc-a2': TEXT_A, 'wb208-doc-b1': TEXT_B}
        with db:
            for asset, title, platform, created, archived in [
                ('wb208-a', '拖延学习：十分钟计时器😀', 'douyin', 300, None),
                ('wb208-b', '拖延学习：把模糊清单改成动作', 'douyin', 200, None),
                ('wb208-other', '厨房收纳的另一类资料', 'youtube', 100, None),
                ('wb208-archived', '已归档的拖延学习', 'douyin', 400, 500),
            ]:
                db.execute('INSERT INTO library_assets(id,title,author,platform,video_id,canonical_url,created_at,updated_at,archived_at) VALUES(?,?,?,?,?,?,?,?,?)',
                           (asset, title, '合成验收作者', platform, asset, 'https://example.invalid/PRIVATE_WB208_SIGNED_URL', created, created, archived))
            db.executemany('INSERT INTO library_categories(id,parent_id,name,position) VALUES(?,?,?,?)', [
                ('wb208-learning', None, '创作学习', 0), ('wb208-hook', 'wb208-learning', '开场钩子', 0), ('wb208-other-category', None, '生活收纳', 1)])
            db.executemany('INSERT INTO library_tags(id,name) VALUES(?,?)', [('wb208-tag', 'WB208验收'), ('wb208-other-tag', '收纳')])
            db.executemany('INSERT INTO asset_categories VALUES(?,?)', [('wb208-a', 'wb208-learning'), ('wb208-b', 'wb208-hook'), ('wb208-other', 'wb208-other-category'), ('wb208-archived', 'wb208-learning')])
            db.executemany('INSERT INTO asset_tags VALUES(?,?)', [('wb208-a', 'wb208-tag'), ('wb208-b', 'wb208-tag'), ('wb208-other', 'wb208-other-tag'), ('wb208-archived', 'wb208-tag')])
            for asset in ['wb208-a', 'wb208-b']:
                path = media / f'PRIVATE_WB208_PATH_{asset}.mp4'
                shutil.copyfile(ROOT / 'tests/fixtures/media/player-portrait.mp4', path)
                db.execute('INSERT INTO asset_files(id,asset_id,path,size,mtime_ns) VALUES(?,?,?,?,?)', (f'{asset}-file', asset, str(path.resolve()), path.stat().st_size, str(path.stat().st_mtime_ns)))
            for document, asset, version, text in [
                ('wb208-doc-a1', 'wb208-a', 1, '过时整理稿：只要一直催自己，就一定能完成任务。'),
                ('wb208-doc-a2', 'wb208-a', 2, TEXT_A), ('wb208-doc-b1', 'wb208-b', 1, TEXT_B),
            ]:
                db.execute("INSERT INTO asset_documents(id,asset_id,kind,text,version,revision,source_document_id,created_at) VALUES(?,?,'edited',?,?,?, ?,?)", (document, asset, text, version, version, 'wb208-doc-a1' if version == 2 else None, 600 + version))
            for asset, document, body in [
                ('wb208-a', 'wb208-doc-a2', '人工观察：计时器近景开头，随后是清单俯拍；用可见进度降低开始的阻力。这里没有逐帧时间戳。'),
                ('wb208-b', 'wb208-doc-b1', '人工观察：先展示左右清单，再随口播动作切到勾选近景；用对照说明差异。这里没有逐帧时间戳。'),
            ]:
                quote = texts[document][0:20]
                citations = json.dumps([{'asset_id': asset, 'document_id': document, 'quote': quote}], ensure_ascii=False)
                db.execute("INSERT INTO asset_notes(id,asset_id,kind,body,origin,citations_json,revision,created_at,updated_at) VALUES(?,?,'note',?,'human',?,1,700,700)", (f'{asset}-note', asset, body, citations))
        expected = {'purpose': 'wb208-synthetic', 'assets': ['wb208-a', 'wb208-b'], 'category_id': 'wb208-learning', 'tag_id': 'wb208-tag', 'documents': [
            {'asset_id': 'wb208-a', 'document_id': 'wb208-doc-a2', 'version': 2, 'text': TEXT_A},
            {'asset_id': 'wb208-b', 'document_id': 'wb208-doc-b1', 'version': 1, 'text': TEXT_B},
        ]}
        (output / 'expected.json').write_text(json.dumps(expected, ensure_ascii=False, indent=2), encoding='utf-8')
        return expected
    finally:
        db.close()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--data', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--snapshot', action='store_true')
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    if args.snapshot:
        print(json.dumps(snapshot(args.data), ensure_ascii=False))
    else:
        seed(args.data, args.output)
        print('WB208 synthetic library seeded: 2 target videos, 1 excluded, 1 archived')
