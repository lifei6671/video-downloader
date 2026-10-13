"""Synchronize a stable vMAJOR.MINOR.PATCH tag in a disposable CI checkout."""
import json
import re
import sys
from pathlib import Path


def prepare(root: Path, tag: str) -> str:
    if not re.fullmatch(r'v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)', tag):
        raise ValueError('Release tag must be vMAJOR.MINOR.PATCH, for example v1.2.3')
    version = tag[1:]
    if any(int(part) > 65535 for part in version.split('.')):
        raise ValueError('Windows installer version components must not exceed 65535')
    for name in ('package.json', 'package-lock.json', 'src-tauri/tauri.conf.json'):
        path = root / name
        data = json.loads(path.read_text(encoding='utf-8'))
        data['version'] = version
        if name == 'package-lock.json':
            data['packages']['']['version'] = version
        path.write_text(json.dumps(data, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
    for name, pattern in (
        ('src-tauri/Cargo.toml', r'(\[package\]\s*name = "yingqu-core"\s*version = ")[^"]+(")'),
        ('src-tauri/Cargo.lock', r'(\[\[package\]\]\s*name = "yingqu-core"\s*version = ")[^"]+(")'),
    ):
        path = root / name
        content, count = re.subn(pattern, lambda m: m[1] + version + m[2], path.read_text(encoding='utf-8'))
        if count != 1:
            raise ValueError(f'Expected one yingqu-core package version in {name}')
        path.write_text(content, encoding='utf-8')
    return version


if __name__ == '__main__':
    print(prepare(Path(__file__).resolve().parent.parent, sys.argv[1]))
