from pathlib import Path

import pytest

import local_tools
import prepare_test_media as media


def test_existing_tools_are_exported_without_downloading(tmp_path, monkeypatch):
    path_file = tmp_path / 'github-path'
    monkeypatch.setenv('GITHUB_PATH', str(path_file))
    monkeypatch.setattr(local_tools, 'installed_tool', lambda name: str(tmp_path / 'existing tools' / name))
    monkeypatch.setattr(local_tools, 'tool_output', lambda path, flag: 'tool version fixture\n')
    monkeypatch.setattr(media.runtime_dependencies, 'install', lambda *args: pytest.fail('unexpected download'))
    media.prepare()
    assert path_file.read_text().splitlines() == [str(tmp_path / 'existing tools')]


@pytest.mark.parametrize('failure', [None, 'checksum', 'missing-ffprobe'])
def test_missing_tools_use_existing_installer_and_publish_only_after_validation(tmp_path, monkeypatch, failure):
    path_file = tmp_path / 'github-path'
    monkeypatch.setenv('GITHUB_PATH', str(path_file))
    monkeypatch.setenv('RUNNER_TEMP', str(tmp_path))
    monkeypatch.setattr(local_tools, 'MANAGED_DIR', None)
    monkeypatch.setattr(local_tools, 'installed_tool', lambda name:
                        str(local_tools.MANAGED_DIR / 'ffmpeg' / name)
                        if local_tools.MANAGED_DIR and (local_tools.MANAGED_DIR / 'ffmpeg' / name).is_file() else None)
    monkeypatch.setattr(local_tools, 'tool_output', lambda path, flag: 'tool version fixture\n')
    def install(tool, stage, emit):
        assert tool == 'ffmpeg'
        assert Path(stage).parent == tmp_path / 'yingqu-test-media'
        assert not any(Path(stage).iterdir())
        if failure == 'checksum':
            (Path(stage) / 'partial').write_bytes(b'partial')
            raise media.runtime_dependencies.DependencyError('DEPENDENCY_CHECKSUM_MISMATCH')
        target = local_tools.MANAGED_DIR / 'ffmpeg'
        target.mkdir()
        for name in ('ffmpeg',) if failure == 'missing-ffprobe' else ('ffmpeg', 'ffprobe'):
            (target / name).write_bytes(b'fixture')
    monkeypatch.setattr(media.runtime_dependencies, 'install', install)
    if failure:
        with pytest.raises((RuntimeError, media.runtime_dependencies.DependencyError),
                           match='ffprobe' if failure == 'missing-ffprobe' else 'CHECKSUM_MISMATCH'):
            media.prepare()
        assert not path_file.exists()
    else:
        media.prepare()
        assert path_file.read_text().splitlines() == [str(tmp_path / 'yingqu-test-media' / 'ffmpeg')]
    assert not list((tmp_path / 'yingqu-test-media').glob('install-*'))


def test_check_only_reports_missing_dependency_without_installing(tmp_path, monkeypatch):
    monkeypatch.delenv('RUNNER_TEMP', raising=False)
    monkeypatch.delenv('GITHUB_PATH', raising=False)
    monkeypatch.setattr(local_tools, 'installed_tool', lambda name: None)
    monkeypatch.setattr(media.runtime_dependencies, 'install', lambda *args: pytest.fail('unexpected download'))
    with pytest.raises(RuntimeError, match='Media tests require working ffmpeg, ffprobe'):
        media.prepare(check_only=True)
