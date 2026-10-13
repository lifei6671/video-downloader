"""Run the M1 probe; credentials travel over stdin, never command arguments."""
import argparse
import json
import subprocess
import sys
from datetime import datetime
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
EDGE_UA = ('Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 '
           '(KHTML, like Gecko) Chrome/150.0.0.0 Safari/537.36 Edg/150.0.0.0')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('config', nargs='?', type=Path, help='Optional existing JSON config')
    parser.add_argument('--url', help='TikTok link or share text')
    parser.add_argument('--cookie-file', type=Path)
    network = parser.add_mutually_exclusive_group()
    network.add_argument('--proxy', help='Proxy URL; default socks5://127.0.0.1:3067')
    network.add_argument('--direct', action='store_true', help='Disable proxy')
    parser.add_argument('--user-agent')
    parser.add_argument('--output-dir', type=Path)
    parser.add_argument('--probe', type=Path, default=ROOT / 'src-tauri/target/debug' /
                        ('m1-probe.exe' if sys.platform == 'win32' else 'm1-probe'))
    parser.add_argument('--engine', type=Path, help='Packaged engine; default current Python')
    parser.add_argument('--script', type=Path)
    args = parser.parse_args()
    if args.config and any((args.url, args.cookie_file, args.proxy, args.direct,
                            args.user_agent, args.output_dir)):
        parser.error('Do not combine a config file with TikTok input/profile options')
    engine = args.engine or Path(sys.executable)
    script = args.script or (None if args.engine else ROOT / 'python-engine/bridge.py')
    if not args.probe.is_file():
        parser.exit(2, 'Probe missing. Run: cargo build --manifest-path src-tauri/Cargo.toml --bin m1-probe\n')
    if not engine.is_file() or (script and not script.is_file()):
        parser.exit(2, 'Engine executable or script not found.\n')
    try:
        if args.config:
            config = json.loads(args.config.read_text(encoding='utf-8-sig'))
        else:
            config = {
                'inputs': [args.url or 'https://vt.tiktok.com/ZSbEe5Vug/'],
                'profiles': {'tiktok': {
                    'cookie_file': str(args.cookie_file or ROOT / 'tests/TikTok.txt'),
                    'proxy': None if args.direct else args.proxy or 'socks5://127.0.0.1:3067',
                    'user_agent': args.user_agent or EDGE_UA,
                }},
                'output_dir': str(args.output_dir or ROOT / '.local' /
                                  ('manual-' + datetime.now().strftime('%Y%m%d-%H%M%S-%f'))),
            }
        for profile in config['profiles'].values():
            cookie_file = profile.pop('cookie_file', None)
            if cookie_file:
                profile['cookie'] = Path(cookie_file).read_text(encoding='utf-8-sig').strip()
        Path(config['output_dir']).mkdir(parents=True, exist_ok=True)
    except (OSError, ValueError, KeyError, TypeError, AttributeError):
        parser.exit(2, 'Cannot read config/Cookie or create output directory. Check paths and JSON format.\n')
    command = [str(args.probe.resolve()), str(engine.resolve())]
    if script:
        command.append(str(script.resolve()))
    print('Starting parse -> download. Each invocation reads the current Cookie file.', flush=True)
    try:
        result = subprocess.run(command, input=json.dumps(config).encode('utf-8'))
    except KeyboardInterrupt:
        return 130
    except OSError:
        parser.exit(2, 'Cannot start probe. Check the executable.\n')
    print('PASS: download completed.' if result.returncode == 0 else
          'FAIL: see the error code above. No automatic retry.', flush=True)
    return result.returncode


if __name__ == '__main__':
    raise SystemExit(main())
