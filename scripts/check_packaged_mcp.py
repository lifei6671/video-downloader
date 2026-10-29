"""Check the desktop HTTP MCP integration and complete notices in a built app/install tree."""
import argparse
import os
from pathlib import Path


def verify_windows_binaries(installed, built, main_binary):
    # Tauri 2.11.4 patches the first bundle marker for NSIS, then restores the
    # unpatched build output. Compare against precisely that shipped variant.
    # https://github.com/tauri-apps/tauri/blob/tauri-cli-v2.11.4/crates/tauri-bundler/src/bundle.rs
    main = (built / main_binary).read_bytes()
    token = b'__TAURI_BUNDLE_TYPE_VAR_UNK'
    if token not in main:
        raise ValueError('Missing Tauri build bundle marker')
    expected = main.replace(token, b'__TAURI_BUNDLE_TYPE_VAR_NSS', 1)
    if (installed / main_binary).read_bytes() != expected:
        raise ValueError('Installed main binary differs beyond the Tauri NSIS marker')


def check_app(app, main_binary=None, build_dir=None):
    app = app.resolve(strict=True)
    mac = app.suffix == '.app'
    binaries = app / 'Contents/MacOS' if mac else app
    resources = app / 'Contents/Resources' if mac else app
    main = binaries / (main_binary or ('yingqu.exe' if os.name == 'nt' else 'yingqu'))
    # Tauri renames the primary macOS binary to the product name.
    if mac:
        import plistlib
        main = binaries / plistlib.loads((app / 'Contents/Info.plist').read_bytes())['CFBundleExecutable']
    notices = resources / 'licenses/mcp-licenses.txt'
    for file in [main, notices]:
        if not file.is_file() or file.stat().st_size == 0:
            raise ValueError(f'Missing packaged MCP file: {file}')
    source_notices = Path(__file__).resolve().parent.parent / 'vendor/mcp-licenses.txt'
    if notices.read_bytes() != source_notices.read_bytes():
        raise ValueError('Packaged MCP licenses do not match the reviewed source')
    if build_dir is not None:
        verify_windows_binaries(binaries, build_dir.resolve(strict=True), main.name)
    print(f'Packaged desktop HTTP MCP files verified: {main}')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('app', type=Path)
    parser.add_argument('--main-binary', help='Only for an isolated acceptance build with a renamed main binary')
    parser.add_argument('--build-dir', type=Path, help='Windows NSIS byte comparison against the build outputs')
    args = parser.parse_args()
    check_app(args.app, args.main_binary, args.build_dir)
