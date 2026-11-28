import hashlib
import json
from datetime import datetime
from pathlib import Path

import pytest

from generate_update_manifest import generate


def release_files(tmp_path):
    notes = tmp_path / 'docs/releases/v1.2.0.md'
    notes.parent.mkdir(parents=True)
    notes.write_text('## 更新说明\n- 修复下载问题\n', encoding='utf-8')
    installers = tmp_path / 'installers'
    installers.mkdir()
    for name in ('Yingqu-windows-x64-setup-1.2.0.exe',
                 'Yingqu-darwin-x64-1.2.0.dmg', 'Yingqu-darwin-aarch64-1.2.0.dmg'):
        (installers / name).write_bytes(name.encode())
    return notes, installers


def test_generator_matches_client_contract_and_actual_installer_bytes(tmp_path):
    _, installers = release_files(tmp_path)
    manifest = generate(tmp_path, 'v1.2.0', installers)
    assert datetime.fromisoformat(manifest['published_at']).utcoffset().total_seconds() == 0
    fixture = json.loads((Path(__file__).parent / 'fixtures/update-manifest.json').read_text(encoding='utf-8'))
    manifest['published_at'] = fixture['published_at']
    assert manifest == fixture
    for asset in manifest['assets']:
        data = (installers / asset['name']).read_bytes()
        assert asset['size'] == len(data)
        assert asset['digest'] == 'sha256:' + hashlib.sha256(data).hexdigest()


@pytest.mark.parametrize('fault', ['missing', 'empty', 'extra', 'notes', 'tag'])
def test_invalid_release_inputs_never_produce_a_manifest(tmp_path, fault):
    notes, installers = release_files(tmp_path)
    windows = installers / 'Yingqu-windows-x64-setup-1.2.0.exe'
    tag = 'v1.2.0'
    if fault == 'missing':
        windows.unlink()
    elif fault == 'empty':
        windows.write_bytes(b'')
    elif fault == 'extra':
        (installers / 'unexpected.exe').write_bytes(b'extra')
    elif fault == 'notes':
        notes.write_text(' \n', encoding='utf-8')
    else:
        tag = 'v1.2.0-rc.1'
    with pytest.raises(ValueError):
        generate(tmp_path, tag, installers)
    assert not (installers / 'update.json').exists()
