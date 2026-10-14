"""Find installed tools, including GUI apps' incomplete/stale PATH environments."""
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

RUNTIME_MINIMUM = {'deno': (2, 3, 0), 'node': (22, 0, 0)}


def windows_path():
    import winreg
    paths = []
    for hive, key in ((winreg.HKEY_CURRENT_USER, 'Environment'),
                      (winreg.HKEY_LOCAL_MACHINE,
                       r'SYSTEM\CurrentControlSet\Control\Session Manager\Environment')):
        try:
            with winreg.OpenKey(hive, key) as handle:
                value = winreg.QueryValueEx(handle, 'Path')[0]
                paths.extend(Path(os.path.expandvars(p.strip('"'))) for p in value.split(';') if p)
        except OSError:
            continue
    return paths


def search_paths(name):
    home = Path.home()
    roots = [home / '.local/bin', home / 'bin']
    if prefix := os.environ.get('CONDA_PREFIX'):
        roots.append(Path(prefix) / ('Library/bin' if sys.platform == 'win32' else 'bin'))
    if sys.platform == 'win32':
        local = Path(os.environ.get('LOCALAPPDATA', home / 'AppData/Local'))
        program_files = Path(os.environ.get('ProgramFiles', 'C:/Program Files'))
        roots += windows_path() + [local / 'Microsoft/WinGet/Links', home / 'scoop/shims',
            Path(os.environ.get('ChocolateyInstall', 'C:/ProgramData/chocolatey')) / 'bin',
            program_files / 'ffmpeg/bin', program_files / 'nodejs', home / '.deno/bin']
        if link := os.environ.get('NVM_SYMLINK'):
            roots.append(Path(link))
        if name == 'ffmpeg':
            # WinGet portable packages can remain installed when their PATH/link is stale.
            packages = local / 'Microsoft/WinGet/Packages'
            roots += [p.parent for pattern in ('*FFmpeg*/bin/ffmpeg.exe', '*FFmpeg*/*/bin/ffmpeg.exe')
                      for p in packages.glob(pattern)]
            roots += [p / 'current/bin' for p in (home / 'scoop/apps').glob('*ffmpeg*')]
    elif sys.platform == 'darwin':
        roots += [Path('/opt/homebrew/bin'), Path('/usr/local/bin'), Path('/opt/local/bin'),
                  Path('/usr/bin'), home / '.nix-profile/bin', Path('/nix/var/nix/profiles/default/bin'),
                  home / '.deno/bin', home / 'miniconda3/bin', home / 'anaconda3/bin']
        if prefix := os.environ.get('HOMEBREW_PREFIX'):
            roots.append(Path(prefix) / 'bin')
        # Read path_helper's inputs; never source shell startup scripts or launch an interactive shell.
        for file in [Path('/etc/paths'), *Path('/etc/paths.d').glob('*')]:
            try:
                roots.extend(Path(line.strip()) for line in file.read_text().splitlines()
                             if line.strip().startswith('/'))
            except (OSError, UnicodeError):
                continue
        for prefix in (Path('/opt/homebrew'), Path('/usr/local')):
            roots.append(prefix / 'opt' / name / 'bin')
            roots += [p / 'bin' for p in (prefix / 'opt').glob(name + '@*') if p.is_dir()]
            formulae = [prefix / 'Cellar' / name, *(prefix / 'Cellar').glob(name + '@*')]
            if name == 'ffmpeg':
                roots.append(prefix / 'opt/ffmpeg-full/bin')
                formulae.append(prefix / 'Cellar/ffmpeg-full')
            roots += [p / 'bin' for formula in formulae for p in formula.glob('*') if p.is_dir()]
        if name == 'node':
            roots += [p / 'bin' for p in (home / '.nvm/versions/node').glob('*') if p.is_dir()]
    return roots


def tool_output(path, flag):
    try:
        result = subprocess.run([str(path), flag], capture_output=True, timeout=5,
                                creationflags=0x08000000 if sys.platform == 'win32' else 0)
        return result.stdout.decode('utf-8', errors='replace') if result.returncode == 0 else ''
    except (OSError, subprocess.SubprocessError):
        return ''


def installed_tool(name):
    found = shutil.which(name)
    filename = name + ('.exe' if sys.platform == 'win32' else '')
    paths = ([Path(found)] if found else []) + [root / filename for root in search_paths(name)]
    seen = set()
    for path in paths:
        identity = os.path.normcase(str(path))
        if identity in seen or not path.is_file():
            continue
        seen.add(identity)
        # Skip stale links, wrong architecture, denied execution, and unrelated same-name files.
        if name == 'ffmpeg':
            if not re.match(r'ffmpeg version\s', tool_output(path, '-version')):
                continue
        elif name in RUNTIME_MINIMUM:
            version = re.search(r'(\d+)\.(\d+)\.(\d+)', tool_output(path, '--version'))
            if not version or tuple(map(int, version.groups())) < RUNTIME_MINIMUM[name]:
                continue
        return str(path)
    return None


def javascript_runtime():
    for name in RUNTIME_MINIMUM:
        path = installed_tool(name)
        if path:
            return {name: {'path': path}}
    return {}
