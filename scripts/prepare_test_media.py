"""Prepare existing pinned FFmpeg dependencies for CI media tests."""
import argparse
import os
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'python-engine'))
import local_tools
import runtime_dependencies


def prepare(check_only=False):
    if not check_only and not local_tools.installed_tool('ffmpeg'):
        root = Path(os.environ['RUNNER_TEMP']).resolve() / 'yingqu-test-media'
        root.mkdir(exist_ok=True)
        runtime_dependencies.configure(str(root))
        with tempfile.TemporaryDirectory(prefix='install-', dir=root) as stage:
            runtime_dependencies.install('ffmpeg', stage,
                                         lambda event, **data: print(event, data.get('phase', ''), flush=True))

    tools = {name: local_tools.installed_tool(name) for name in ('ffmpeg', 'ffprobe')}
    missing = [name for name, path in tools.items() if not path]
    if missing:
        raise RuntimeError('Media tests require working ' + ', '.join(missing)
                           + '; install FFmpeg locally or run scripts/prepare_test_media.py in CI')
    if not check_only:
        with open(os.environ['GITHUB_PATH'], 'a', encoding='utf-8') as output:
            for directory in sorted({str(Path(path).parent) for path in tools.values()}):
                output.write(directory + '\n')
    for name, path in tools.items():
        print(f'{name}: {local_tools.tool_output(path, "-version").splitlines()[0]}', flush=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true', help='Only check installed tools; do not download or change PATH')
    prepare(check_only=parser.parse_args().check)
