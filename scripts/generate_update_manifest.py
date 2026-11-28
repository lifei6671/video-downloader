"""Build the static update.json from the three final release installers."""
import hashlib
import json
import sys
from datetime import datetime, timezone
from pathlib import Path

from prepare_release import release_version


def generate(root: Path, tag: str, installers: Path) -> dict:
    version = release_version(tag)
    notes = (root / 'docs' / 'releases' / f'{tag}.md').read_text(encoding='utf-8').strip()
    if not notes:
        raise ValueError('Release notes must not be empty')
    names = (
        f'Yingqu-windows-x64-setup-{version}.exe',
        f'Yingqu-darwin-x64-{version}.dmg',
        f'Yingqu-darwin-aarch64-{version}.dmg',
    )
    actual = {path.name for path in installers.iterdir() if path.suffix in ('.exe', '.dmg')}
    if actual != set(names):
        raise ValueError('Expected exactly one installer for each release platform')
    assets = []
    for name in names:
        path = installers / name
        size = path.stat().st_size
        if not path.is_file() or not 0 < size <= 1024 ** 3:
            raise ValueError(f'Invalid installer: {name}')
        with path.open('rb') as stream:
            digest = hashlib.file_digest(stream, 'sha256').hexdigest()
        assets.append({
            'name': name, 'size': size, 'state': 'uploaded',
            'digest': f'sha256:{digest}',
            'browser_download_url': f'https://github.com/lifei6671/video-downloader/releases/download/{tag}/{name}',
        })
    return {
        'schema_version': 1, 'tag_name': tag, 'draft': False, 'prerelease': False,
        'body': notes, 'published_at': datetime.now(timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ'),
        'assets': assets,
    }


if __name__ == '__main__':
    directory = Path(sys.argv[2])
    manifest = generate(Path(__file__).resolve().parent.parent, sys.argv[1], directory)
    output = directory / 'update.json'
    output.write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
    print(output)
