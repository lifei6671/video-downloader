"""Application-local dependencies. Uses the same discovery as media operations."""
import gzip
import hashlib
import os
import platform
import re
import shutil
import ssl
import subprocess
import sys
import time
import zipfile
from pathlib import Path
from urllib.error import URLError
from urllib.request import HTTPRedirectHandler, Request, build_opener, HTTPSHandler

import certifi
import local_tools
from runtime_assets import assets


class DependencyError(Exception):
    pass


def configure(directory):
    if directory is not None:
        if not isinstance(directory, str) or not Path(directory).is_absolute():
            raise DependencyError('ENGINE_PROTOCOL_ERROR')
        local_tools.MANAGED_DIR = Path(directory)


def platform_key():
    arch = {'amd64': 'x64', 'x86_64': 'x64', 'arm64': 'arm64', 'aarch64': 'arm64'}.get(platform.machine().lower(), 'unknown')
    return f'{sys.platform}-{arch}'


def status():
    root = local_tools.MANAGED_DIR
    tools = []
    runtime = local_tools.javascript_runtime()
    for tool, name in [('ffmpeg', 'ffmpeg'), ('javascript', next(iter(runtime), 'deno'))]:
        path = local_tools.installed_tool('ffmpeg') if tool == 'ffmpeg' else runtime.get(name, {}).get('path')
        output = local_tools.tool_output(path, '-version' if name == 'ffmpeg' else '--version') if path else ''
        version = re.search(r'(?:ffmpeg version\s+|deno\s+|^v)([^\s]+)', output)
        found = bool(path and version)
        tools.append(dict(id=tool, available=found, provider=name if found else None,
                          version=version[1] if found else None, path=path if found else None,
                          source=('managed' if Path(path).resolve().is_relative_to(root.resolve()) else 'system') if found else None,
                          can_install=bool(assets(tool, platform_key()))))
    return dict(platform=platform_key(), directory=str(root), tools=tools)


class DownloadRedirects(HTTPRedirectHandler):
    def redirect_request(self, request, fp, code, msg, headers, newurl):
        from urllib.parse import urlsplit
        parsed = urlsplit(newurl)
        if (parsed.scheme != 'https' or parsed.port not in (None, 443)
                or parsed.hostname not in ('github.com', 'release-assets.githubusercontent.com', 'objects.githubusercontent.com', 'raw.githubusercontent.com')):
            raise DependencyError('DEPENDENCY_NETWORK_ERROR')
        return super().redirect_request(request, fp, code, msg, headers, newurl)


def copy_binary(source, destination):
    size = 0
    with open(destination, 'wb') as output:
        while chunk := source.read(1024 * 1024):
            size += len(chunk)
            if size > 256 * 1024 * 1024:
                raise DependencyError('DEPENDENCY_INVALID_ARCHIVE')
            output.write(chunk)


def install(tool, staging, emit):
    if tool not in ('ffmpeg', 'javascript'):
        raise DependencyError('ENGINE_PROTOCOL_ERROR')
    current = status()
    if next(row for row in current['tools'] if row['id'] == tool)['available']:
        return current
    downloads = assets(tool, platform_key())
    if not downloads:
        raise DependencyError('DEPENDENCY_UNSUPPORTED_SYSTEM')
    stage = Path(staging)
    root = local_tools.MANAGED_DIR
    if not stage.is_dir() or stage.is_symlink() or stage.parent != root or any(stage.iterdir()):
        raise DependencyError('IO_ERROR')
    prepared = stage / 'prepared'
    prepared.mkdir()
    # Use the system's proxy, never a platform Cookie or credential snapshot.
    context = ssl.create_default_context(cafile=certifi.where())
    opener = build_opener(HTTPSHandler(context=context), DownloadRedirects())
    total = sum(item[3] for item in downloads)
    received, last = 0, 0
    deadline = time.monotonic() + 300

    def progress(phase):
        emit('dependency_progress', tool=tool, phase=phase, downloaded_bytes=received, total_bytes=total)

    try:
        progress('downloading')
        for index, (url, filename, compression, size, digest) in enumerate(downloads):
            archive = stage / f'{index}.part'
            checksum, count = hashlib.sha256(), 0
            request = Request(url, headers={'User-Agent': 'Yingqu-runtime/1', 'Accept-Encoding': 'identity'})
            with opener.open(request, timeout=15) as response, archive.open('wb') as output:
                if response.status != 200:
                    raise DependencyError('DEPENDENCY_NETWORK_ERROR')
                while chunk := response.read(128 * 1024):
                    if time.monotonic() > deadline:
                        raise DependencyError('DEPENDENCY_TIMEOUT')
                    count += len(chunk)
                    received += len(chunk)
                    if count > size:
                        raise DependencyError('DEPENDENCY_CHECKSUM_MISMATCH')
                    checksum.update(chunk)
                    output.write(chunk)
                    if time.monotonic() - last >= 0.2:
                        progress('downloading')
                        last = time.monotonic()
            if count != size or checksum.hexdigest() != digest:
                raise DependencyError('DEPENDENCY_CHECKSUM_MISMATCH')
            progress('verifying')
            target = prepared / filename
            if compression == 'gzip':
                with gzip.open(archive, 'rb') as source:
                    copy_binary(source, target)
            elif compression == 'zip':
                with zipfile.ZipFile(archive) as package:
                    # Read only the expected executable, never extract arbitrary members.
                    with package.open(filename) as source:
                        copy_binary(source, target)
            else:
                shutil.copyfile(archive, target)
        binaries = ['ffmpeg', 'ffprobe'] if tool == 'ffmpeg' else ['deno']
        progress('installing')
        for name in binaries:
            file = prepared / (name + ('.exe' if sys.platform == 'win32' else ''))
            file.chmod(0o755)
            if sys.platform == 'darwin':
                signed = subprocess.run(['/usr/bin/codesign', '--force', '--sign', '-', str(file)],
                                        capture_output=True, timeout=15)
                if signed.returncode:
                    raise DependencyError('DEPENDENCY_VALIDATION_FAILED')
            output = local_tools.tool_output(file, '--version' if name == 'deno' else '-version')
            pattern = r'^deno 2\.9\.7(?:\s|$)' if name == 'deno' else rf'^{name} version 6\.1\.1(?:\s|$|-)'
            if not re.match(pattern, output):
                raise DependencyError('DEPENDENCY_VALIDATION_FAILED')
        (prepared / 'SOURCE.txt').write_text('\n'.join(url for url, *_ in downloads) + '\n', encoding='utf-8')
        target = root / tool
        backup = stage / 'previous'
        if target.exists():
            target.rename(backup)
        try:
            prepared.rename(target)
        except OSError:
            if backup.exists():
                backup.rename(target)
            raise
        result = status()
        if not next(row for row in result['tools'] if row['id'] == tool)['available']:
            raise DependencyError('DEPENDENCY_VALIDATION_FAILED')
        return result
    except (TimeoutError, subprocess.TimeoutExpired):
        raise DependencyError('DEPENDENCY_TIMEOUT') from None
    except URLError:
        raise DependencyError('DEPENDENCY_NETWORK_ERROR') from None
    except (gzip.BadGzipFile, zipfile.BadZipFile, KeyError, EOFError):
        raise DependencyError('DEPENDENCY_INVALID_ARCHIVE') from None
