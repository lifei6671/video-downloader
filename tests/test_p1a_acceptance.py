"""The macOS launcher must reject profiles it does not own before seeding/building."""
import importlib.util
import json
from pathlib import Path

import pytest


spec = importlib.util.spec_from_file_location(
    'p1a_macos', Path(__file__).resolve().parents[1] / 'scripts/p1a-acceptance-macos.py')
launcher = importlib.util.module_from_spec(spec)
spec.loader.exec_module(launcher)


@pytest.mark.parametrize('marker', [None, {'identifier': 'another-profile'}])
def test_launcher_refuses_unowned_profile_before_any_subprocess(tmp_path, monkeypatch, marker):
    monkeypatch.setattr(launcher.sys, 'platform', 'darwin')
    monkeypatch.setattr(launcher.sys, 'argv', ['launcher', '--run-id', 'check'])
    monkeypatch.setattr(launcher.Path, 'home', lambda: tmp_path)
    def unexpected(*args, **kwargs):
        pytest.fail('Refused profiles must not seed or build')
    monkeypatch.setattr(launcher.subprocess, 'run', unexpected)
    data = tmp_path / 'Library/Application Support/me.disign.yingqu.p1a-check-empty'
    data.mkdir(parents=True)
    sentinel = data / 'keep.txt'
    sentinel.write_text('existing user data')
    if marker is not None:
        (data / 'p1a-acceptance.json').write_text(json.dumps(marker))
    with pytest.raises(ValueError, match='unmarked|does not match'):
        launcher.main()
    assert sentinel.read_text() == 'existing user data'
    assert not (data / 'yingqu.sqlite').exists()
