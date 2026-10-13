import json
import shutil
import tomllib
from pathlib import Path

import pytest

from prepare_release import prepare


def test_release_versions_match_tag_and_preserve_dependency_lock(tmp_path):
    root = Path(__file__).resolve().parent.parent
    names = ('package.json', 'package-lock.json', 'src-tauri/tauri.conf.json',
             'src-tauri/Cargo.toml', 'src-tauri/Cargo.lock')
    for name in names:
        destination = tmp_path / name
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(root / name, destination)
    original = tomllib.loads((tmp_path / 'src-tauri/Cargo.lock').read_text())
    assert prepare(tmp_path, 'v1.2.3') == '1.2.3'
    for name in names[:3]:
        assert json.loads((tmp_path / name).read_text(encoding='utf-8'))['version'] == '1.2.3'
    assert json.loads((tmp_path / 'package-lock.json').read_text())['packages']['']['version'] == '1.2.3'
    assert tomllib.loads((tmp_path / 'src-tauri/Cargo.toml').read_text())['package']['version'] == '1.2.3'
    updated = tomllib.loads((tmp_path / 'src-tauri/Cargo.lock').read_text())
    package = next(p for p in updated['package'] if p['name'] == 'yingqu-core')
    assert package['version'] == '1.2.3'
    package['version'] = next(p for p in original['package'] if p['name'] == 'yingqu-core')['version']
    assert updated == original


@pytest.mark.parametrize('tag', ['main', '1.2.3', 'v01.2.3', 'v1.2', 'v1.2.3-rc.1', 'v65536.0.0', 'v1.2.3\n'])
def test_invalid_tag_does_not_write_files(tmp_path, tag):
    with pytest.raises(ValueError):
        prepare(tmp_path, tag)
    assert not list(tmp_path.iterdir())
