from pathlib import Path
from importlib.metadata import distribution
import sys
import sysconfig

root = Path(SPECPATH).parent
if not (root / 'python-engine/asr' / ('llama-funasr-cli.exe' if sys.platform == 'win32' else 'llama-funasr-cli')).is_file():
    raise RuntimeError('Build the pinned ASR helper first: python scripts/build_asr.py')
pyinstaller = distribution('pyinstaller')
bootloader_license = next(pyinstaller.locate_file(p) for p in pyinstaller.files
                          if str(p).endswith('licenses/COPYING.txt'))
python_license = Path(sys.base_prefix) / 'LICENSE.txt'
if not python_license.is_file():
    python_license = Path(sysconfig.get_path('stdlib')) / 'LICENSE.txt'
runtime_licenses = []
for name in ('pydantic', 'pydantic-core', 'annotated-types', 'typing-inspection', 'typing-extensions',
             'httpx', 'httpcore', 'anyio', 'certifi', 'idna', 'h11', 'structlog', 'yt-dlp-ejs', 'pillow'):
    package = distribution(name)
    for file in package.files or []:
        if 'license' in str(file).lower() or file.name.lower().startswith('notice'):
            path = package.locate_file(file)
            if path.is_file():
                runtime_licenses.append((str(path), 'licenses/' + name))

a = Analysis([str(root / 'python-engine' / 'bridge.py')],
             pathex=[str(root / 'python-engine'), str(root / 'vendor/dtk/upstream/src')],
             binaries=[], datas=[
                 (str(root / 'python-engine/asr'), 'asr'),
                 (str(root / 'vendor/dtk/upstream/LICENSE'), 'licenses/dtk'),
                 (str(root / 'vendor/dtk/upstream/NOTICE'), 'licenses/dtk'),
                 (str(root / 'vendor/yt-dlp-LICENSE'), 'licenses/yt-dlp'),
                 (str(python_license), 'licenses/python'),
                 (str(bootloader_license), 'licenses/pyinstaller'),
                 (str(root / 'LICENSE'), 'licenses/project'),
             ] + runtime_licenses, hiddenimports=['dtk.platforms.douyin.adapter', 'dtk.platforms.tiktok.adapter'],
             hookspath=[], hooksconfig={}, runtime_hooks=[], excludes=[])
pyz = PYZ(a.pure)
exe = EXE(pyz, a.scripts, [], exclude_binaries=True, name='video-engine',
          debug=False, bootloader_ignore_signals=False, strip=False, upx=False,
          console=True, disable_windowed_traceback=False)
coll = COLLECT(exe, a.binaries, a.datas, strip=False, upx=False, name='video-engine')
