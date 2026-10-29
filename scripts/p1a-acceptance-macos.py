"""Build a marked macOS P1A profile using the production Tauri entry point."""
import argparse
import json
import os
from pathlib import Path
import plistlib
import re
import shutil
import subprocess
import sys


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--scenario', choices=['empty', 'upgrade', 'restore'], default='empty')
    parser.add_argument('--run-id', required=True)
    parser.add_argument('--viewport', choices=['1440x900', '1280x800', '1024x680'], default='1440x900')
    args = parser.parse_args()
    if sys.platform != 'darwin':
        parser.error('macOS only; use p1a-acceptance.ps1 on Windows')
    if not re.fullmatch('[a-z0-9-]{1,48}', args.run_id):
        parser.error('run-id must contain 1–48 lowercase letters, digits or hyphens')
    workspace = Path(__file__).resolve().parents[1]
    identifier = f'me.disign.yingqu.p1a-{args.run_id}-{args.scenario}'
    data = Path.home() / 'Library/Application Support' / identifier
    output = workspace / '.local/p1a-acceptance' / args.run_id / args.scenario
    marker = data / 'p1a-acceptance.json'
    expected = dict(identifier=identifier, workspace=str(workspace), scenario=args.scenario)
    if data.exists() and not marker.is_file():
        raise ValueError(f'Refusing unmarked profile: {data}')
    if marker.exists() and json.loads(marker.read_text()) != expected:
        raise ValueError(f'Profile marker does not match this run: {data}')
    data.mkdir(parents=True, exist_ok=True)
    output.mkdir(parents=True, exist_ok=True)
    marker.write_text(json.dumps(expected), encoding='utf-8')
    subprocess.run([sys.executable, str(workspace / 'tests/fixtures/seed_p1a_profile.py'),
                    '--data', str(data), '--output', str(output), '--scenario', args.scenario], check=True)
    width, height = map(int, args.viewport.split('x'))
    config = dict(identifier=identifier, app=dict(windows=[dict(
        label='main', title=f'映取 P1A {args.scenario} {args.viewport} 隔离验收', width=width, height=height,
        minWidth=1024, minHeight=680, decorations=True, theme='Light', backgroundColor='#f9f9fb')]))
    subprocess.run(['cargo', 'build', '--locked', '--manifest-path', str(workspace / 'src-tauri/Cargo.toml'),
                    '--features', 'desktop,mcp', '--example', 'p1a-acceptance', '--bin', 'yingqu-mcp'],
                   cwd=workspace, env={**os.environ, 'TAURI_CONFIG': json.dumps(config)}, check=True)
    # A local debug app, not an installer: the engine and UI use the existing dev paths.
    app = output / '映取隔离验收.app'
    binaries = app / 'Contents/MacOS'
    binaries.mkdir(parents=True, exist_ok=True)
    target = workspace / 'src-tauri/target/debug'
    shutil.copy2(target / 'examples/p1a-acceptance', binaries / 'p1a-acceptance')
    shutil.copy2(target / 'yingqu-mcp', binaries / 'yingqu-mcp')
    with (app / 'Contents/Info.plist').open('wb') as stream:
        plistlib.dump(dict(CFBundleIdentifier=identifier, CFBundleName='映取隔离验收',
                           CFBundleExecutable='p1a-acceptance', CFBundlePackageType='APPL',
                           NSHighResolutionCapable=True, LSMinimumSystemVersion='13.3'), stream)
    profile = dict(**expected, data=str(data), output=str(output), program=str(app), viewport=args.viewport)
    (output / 'profile.json').write_text(json.dumps(profile, ensure_ascii=False, indent=2), encoding='utf-8')
    print(json.dumps(profile, ensure_ascii=False, indent=2))


if __name__ == '__main__':
    main()
