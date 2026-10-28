import importlib.util
import json
import sqlite3
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location('seed_mcp_acceptance', ROOT / 'tests/fixtures/seed_mcp_acceptance.py')
fixture = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixture)


def profile(tmp_path):
    data = tmp_path / 'profile'
    data.mkdir()
    (data / 'wb208-acceptance.json').write_text(json.dumps({'purpose': 'wb208-synthetic', 'workspace': str(ROOT)}), encoding='utf-8')
    with sqlite3.connect(data / 'yingqu.sqlite') as db:
        db.execute('CREATE TABLE download_tasks(id TEXT PRIMARY KEY)')
        for name in ['schema.sql', 'schema-v2.sql', 'schema-v3.sql']:
            db.executescript((ROOT / 'src-tauri/src/library' / name).read_text())
    return data


def test_seeder_refuses_unmarked_profile(tmp_path):
    with pytest.raises(FileNotFoundError):
        fixture.seed(tmp_path, tmp_path / 'output')
    assert not (tmp_path / 'yingqu.sqlite').exists()


def test_seed_and_snapshot_preserve_a_nonempty_library(tmp_path):
    data = profile(tmp_path)
    output = tmp_path / 'output'
    manifest = fixture.seed(data, output)
    assert len(manifest['documents']) == 2
    before = fixture.snapshot(data)
    assert before['library_assets']['rows'] == 4
    assert before['asset_documents']['rows'] == 3
    assert before['asset_notes']['rows'] == 2
    with pytest.raises(ValueError, match='nonempty'):
        fixture.seed(data, output)
    assert fixture.snapshot(data) == before
