"""Sync a pinned DTK parser/signing slice; patches never modify upstream algorithms.

--check is offline. --source uses git blobs from an existing checkout, without
changing it. --ref requires --update to adopt a different audited commit.
"""
import argparse
import ast
import hashlib
import json
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
VENDOR = ROOT / 'vendor' / 'dtk'
SOURCE = VENDOR / 'upstream'
MANIFEST = VENDOR / 'manifest.json'
REPOSITORY = 'https://github.com/Evil0ctal/Douyin_TikTok_Download_API'
FORBIDDEN = ('api', 'worker', 'identity', 'scheduler', 'db', 'transport', 'media', 'cli')


def digest(body):
    return hashlib.sha256(body).hexdigest()


def git(path, *args):
    return subprocess.check_output(['git', '-C', str(path), *args], timeout=90)


def audit(directory, manifest):
    allowed = set(manifest['runtime_imports']) | sys.stdlib_module_names | {'dtk', '__future__'}
    for file in directory.rglob('*.py'):
        tree = ast.parse(file.read_text())
        for node in ast.walk(tree):
            names = ([alias.name for alias in node.names] if isinstance(node, ast.Import)
                     else [node.module] if isinstance(node, ast.ImportFrom) and node.module else [])
            for name in names:
                if name.split('.')[0] not in allowed:
                    raise ValueError(f'Unreviewed dependency: {name}')
                if name.startswith('dtk.'):
                    parts = name.split('.')
                    if parts[1] in FORBIDDEN or name in ('dtk.signing.rpc', 'dtk.signing.registry'):
                        raise ValueError(f'Service import: {name}')
                    target = directory / 'src' / Path(*parts)
                    if not target.with_suffix('.py').is_file() and not (target / '__init__.py').is_file():
                        raise ValueError(f'Unreviewed source import: {name}')
        compile(tree, str(file), 'exec')


def check(manifest, check_patches=True):
    expected = set()
    for item in manifest['files']:
        path = SOURCE / item['path']
        expected.add(item['path'])
        if not path.is_file() or digest(path.read_bytes()) != item['installed_sha256']:
            raise ValueError(f'Vendor modified or missing: {item["path"]}')
    revision = SOURCE / 'src/dtk/_desktop_revision.py'
    expected.add('src/dtk/_desktop_revision.py')
    text = f'COMMIT = {manifest["upstream_commit"]!r}\nVERSION = {manifest["upstream_version"]!r}\n'
    if not revision.is_file() or revision.read_text() != text:
        raise ValueError('Vendor revision differs from manifest')
    actual = {str(p.relative_to(SOURCE)) for p in SOURCE.rglob('*')
              if p.is_file() and '__pycache__' not in p.parts and p.suffix != '.pyc'}
    if actual != expected:
        raise ValueError('Unexpected vendor files')
    for item in manifest['patches'] if check_patches else []:
        if digest((VENDOR / item['path']).read_bytes()) != item['sha256']:
            raise ValueError('Patch modified; review and update its manifest hash')
    audit(SOURCE, manifest)


def sync(source, ref, update):
    manifest = json.loads(MANIFEST.read_text())
    ref = ref or manifest['upstream_commit']
    if not re.fullmatch(r'[0-9a-f]{40}', ref):
        raise ValueError('Use a complete commit SHA')
    if ref != manifest['upstream_commit'] and not update:
        raise ValueError('A new commit requires --update')
    if SOURCE.exists():
        check(manifest, check_patches=not update)  # Never overwrite local edits or an incomplete slice.
    with tempfile.TemporaryDirectory(prefix='dtk-sync-') as temp:
        temp = Path(temp)
        if source is None:
            source = temp / 'repository'
            subprocess.run(['git', 'init', '-q', str(source)], check=True, timeout=30)
            git(source, 'fetch', '--depth', '1', REPOSITORY, ref)
        if git(source, 'rev-parse', ref + '^{commit}').decode().strip() != ref:
            raise ValueError('Source commit does not match')
        staging = temp / 'slice'
        staging.mkdir()
        for item in manifest['files']:
            relative = Path(item['path'])
            if relative.is_absolute() or '..' in relative.parts:
                raise ValueError('Invalid manifest path')
            body = git(source, 'show', ref + ':' + item['path'])
            if not update and digest(body) != item['sha256']:
                raise ValueError(f'Upstream hash changed: {item["path"]}')
            item['sha256'] = digest(body)
            target = staging / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(body)
        for item in manifest['patches']:
            patch = VENDOR / item['path']
            patch_hash = digest(patch.read_bytes())
            if not update and patch_hash != item['sha256']:
                raise ValueError('Patch hash differs from manifest')
            item['sha256'] = patch_hash
            git(staging, 'apply', '--check', str(patch))
            git(staging, 'apply', str(patch))
        init = (staging / 'src/dtk/__init__.py').read_text()
        version = re.search(r'__version__\s*=\s*["\']([^"\']+)["\']', init)
        if not version:
            raise ValueError('Upstream version not found')
        manifest['upstream_commit'] = ref
        manifest['upstream_version'] = version[1]
        for item in manifest['files']:
            item['installed_sha256'] = digest((staging / item['path']).read_bytes())
        (staging / 'src/dtk/_desktop_revision.py').write_text(
            f'COMMIT = {ref!r}\nVERSION = {version[1]!r}\n')
        audit(staging, manifest)
        # Work on a sibling directory so replacement stays on the same volume.
        with tempfile.TemporaryDirectory(prefix='.dtk-staging-', dir=VENDOR) as install:
            candidate = Path(install) / 'upstream'
            shutil.copytree(staging, candidate)
            backup = Path(install) / 'previous'
            if SOURCE.exists():
                SOURCE.rename(backup)
            try:
                candidate.rename(SOURCE)
                new_manifest = Path(install) / 'manifest.json'
                new_manifest.write_text(json.dumps(manifest, indent=2) + '\n')
                new_manifest.replace(MANIFEST)
            except Exception:
                if SOURCE.exists():
                    shutil.rmtree(SOURCE)
                if backup.exists():
                    backup.rename(SOURCE)
                raise
    check(manifest)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true')
    parser.add_argument('--source', type=Path)
    parser.add_argument('--ref')
    parser.add_argument('--update', action='store_true')
    args = parser.parse_args()
    if args.check:
        check(json.loads(MANIFEST.read_text()))
    else:
        sync(args.source, args.ref, args.update)
    print('DTK source, import boundary and patches: PASS')
