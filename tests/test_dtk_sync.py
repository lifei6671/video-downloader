import json
import shutil
import subprocess
import sys
from pathlib import Path

import pytest

import update_dtk_vendor as sync


@pytest.fixture
def vendor(tmp_path, monkeypatch):
    real = sync.VENDOR
    destination = tmp_path / 'vendor'
    shutil.copytree(real, destination, ignore=shutil.ignore_patterns('__pycache__', '*.pyc'))
    repository = tmp_path / 'repository'
    shutil.copytree(destination / 'upstream', repository)
    (repository / 'src/dtk/_desktop_revision.py').unlink()
    subprocess.run(['git', '-C', str(repository), 'apply', '-R', str(real / 'patches/desktop-imports.patch')], check=True)
    subprocess.run(['git', 'init', '-q', str(repository)], check=True)
    monkeypatch.setattr(sync, 'VENDOR', destination)
    monkeypatch.setattr(sync, 'SOURCE', destination / 'upstream')
    monkeypatch.setattr(sync, 'MANIFEST', destination / 'manifest.json')
    return destination, repository


def commit(repository):
    subprocess.run(['git', '-C', str(repository), 'add', '.'], check=True)
    subprocess.run(['git', '-C', str(repository), '-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid', 'commit', '-qm', 'fixture'], check=True)
    return subprocess.check_output(['git', '-C', str(repository), 'rev-parse', 'HEAD'], text=True).strip()


def test_sync_new_commit_tracks_version_and_applies_patch(vendor):
    destination, repository = vendor
    init = repository / 'src/dtk/__init__.py'
    init.write_text(init.read_text(encoding='utf-8').replace(
        json.loads(sync.MANIFEST.read_text(encoding='utf-8'))['upstream_version'], '99.0.0'),
        encoding='utf-8', newline='\n')
    ref = commit(repository)
    sync.sync(repository, ref, True)
    manifest = json.loads(sync.MANIFEST.read_text(encoding='utf-8'))
    assert manifest['upstream_commit'] == ref and manifest['upstream_version'] == '99.0.0'
    assert 'from dtk.signing.rpc' not in (destination / 'upstream/src/dtk/signing/__init__.py').read_text(encoding='utf-8').lower()
    sync.check(manifest)


def test_conflicting_patch_preserves_installed_source_and_manifest(vendor):
    _, repository = vendor
    init = repository / 'src/dtk/signing/__init__.py'
    init.write_text(init.read_text(encoding='utf-8').replace('Request signing:', 'Changed entry:'),
                    encoding='utf-8', newline='\n')
    ref = commit(repository)
    before = sync.MANIFEST.read_bytes()
    with pytest.raises(subprocess.CalledProcessError):
        sync.sync(repository, ref, True)
    assert sync.MANIFEST.read_bytes() == before
    sync.check(json.loads(before))


def test_new_service_import_and_dirty_vendor_are_rejected(vendor):
    _, repository = vendor
    parser = repository / 'src/dtk/platforms/douyin/parser.py'
    parser.write_text(parser.read_text(encoding='utf-8') + '\nimport fastapi\n', encoding='utf-8', newline='\n')
    ref = commit(repository)
    before = sync.MANIFEST.read_bytes()
    with pytest.raises(ValueError, match='Unreviewed dependency'):
        sync.sync(repository, ref, True)
    assert sync.MANIFEST.read_bytes() == before
    installed = sync.SOURCE / 'src/dtk/core/types.py'
    installed.write_text(installed.read_text(encoding='utf-8') + '\n# user edit\n', encoding='utf-8', newline='\n')
    with pytest.raises(ValueError, match='modified'):
        sync.sync(repository, ref, True)


@pytest.mark.parametrize('autocrlf', ['true', 'false'])
def test_clean_checkout_verifies_vendor_and_rejects_modified_bytes(tmp_path, autocrlf):
    repository = tmp_path / 'repository'
    repository.mkdir()
    shutil.copy2(sync.ROOT / '.gitattributes', repository)
    shutil.copytree(sync.VENDOR, repository / 'vendor/dtk',
                    ignore=shutil.ignore_patterns('__pycache__', '*.pyc'))
    (repository / 'scripts').mkdir()
    shutil.copy2(Path(sync.__file__), repository / 'scripts/update_dtk_vendor.py')
    sync.git(repository, 'init', '-q')
    sync.git(repository, 'config', 'core.autocrlf', autocrlf)
    sync.git(repository, 'add', '.')
    checkout = tmp_path / 'checkout'
    sync.git(repository, 'checkout-index', '--all', '--prefix', checkout.as_posix() + '/')
    command = [sys.executable, '-X', 'utf8=0', str(checkout / 'scripts/update_dtk_vendor.py'), '--check']
    result = subprocess.run(command, capture_output=True)
    assert result.returncode == 0, result.stderr
    for relative, error in [
        ('upstream/src/dtk/__init__.py', b'Vendor modified or missing'),
        ('patches/desktop-imports.patch', b'Patch modified'),
    ]:
        path = checkout / 'vendor/dtk' / relative
        original = path.read_bytes()
        assert b'\r\n' not in original
        path.write_bytes(original.replace(b'\n', b'\r\n'))
        result = subprocess.run(command, capture_output=True)
        assert result.returncode != 0 and error in result.stderr
        path.write_bytes(original)
    (checkout / 'vendor/dtk/upstream/unreviewed.txt').write_bytes(b'unexpected')
    result = subprocess.run(command, capture_output=True)
    assert result.returncode != 0 and b'Unexpected vendor files' in result.stderr
