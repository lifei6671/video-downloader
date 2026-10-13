from pathlib import Path
from importlib.metadata import distribution
import sys
import sysconfig

root = Path(SPECPATH).parent
pyinstaller = distribution('pyinstaller')
bootloader_license = next(pyinstaller.locate_file(p) for p in pyinstaller.files
                          if str(p).endswith('licenses/COPYING.txt'))
python_license = Path(sys.base_prefix) / 'LICENSE.txt'
if not python_license.is_file():
    python_license = Path(sysconfig.get_path('stdlib')) / 'LICENSE.txt'
a = Analysis([str(root / 'python-engine' / 'bridge.py')],
             pathex=[str(root / 'python-engine')],
             binaries=[], datas=[
                 (str(root / 'vendor/yt-dlp-LICENSE'), 'licenses/yt-dlp'),
                 (str(python_license), 'licenses/python'),
                 (str(bootloader_license), 'licenses/pyinstaller'),
                 (str(root / 'LICENSE'), 'licenses/project'),
             ], hiddenimports=[],
             hookspath=[], hooksconfig={}, runtime_hooks=[], excludes=[])
pyz = PYZ(a.pure)
exe = EXE(pyz, a.scripts, [], exclude_binaries=True, name='video-engine',
          debug=False, bootloader_ignore_signals=False, strip=False, upx=False,
          console=True, disable_windowed_traceback=False)
coll = COLLECT(exe, a.binaries, a.datas, strip=False, upx=False, name='video-engine')
