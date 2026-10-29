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
import json
import transcription
import nano_cuda
import local_tools
from runtime_assets import assets, FFMPEG_VERSIONS


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
    variant = nano_cuda.selected()
    installed = nano_cuda.available() if variant == 'cuda' else transcription.model_available(root / transcription.MODEL_ID) if variant == 'cpu' else False
    usable = transcription.native_program().is_file() and (variant != 'cuda' or transcription.vad_program().is_file())
    cpu_bytes = sum(row[3] for row in assets(transcription.MODEL_ID, platform_key()))
    cuda_supported = platform_key() == 'win32-x64' and transcription.vad_program().is_file()
    tools.append(dict(id=transcription.MODEL_ID, available=installed and usable,
                      provider=variant, version=(nano_cuda.MODEL_REVISION if variant == 'cuda' else transcription.MODEL_REVISION)[:12],
                      path=str(nano_cuda.directory() if variant == 'cuda' else root / transcription.MODEL_ID), source='managed',
                      can_install=bool(assets(transcription.MODEL_ID, platform_key())) and transcription.native_program().is_file(),
                      download_bytes=nano_cuda.MODEL_SIZE + nano_cuda.VAD_SIZE if variant == 'cuda' else cpu_bytes,
                      variants=['cpu', 'cuda'] if cuda_supported else ['cpu'],
                      variant_download_bytes={'cpu': cpu_bytes, 'cuda': sum(row[3] for row in nano_cuda.downloads()) + nano_cuda.VAD_SIZE},
                      variant_model_bytes={'cpu': cpu_bytes, 'cuda': nano_cuda.MODEL_SIZE + nano_cuda.VAD_SIZE}))
    return dict(platform=platform_key(), directory=str(root), tools=tools)


class DownloadRedirects(HTTPRedirectHandler):
    def __init__(self, model=False):
        super().__init__()
        self.hosts = (('huggingface.co', 'us.aws.cdn.hf.co') if model else
                      ('github.com', 'release-assets.githubusercontent.com', 'objects.githubusercontent.com', 'raw.githubusercontent.com'))

    def redirect_request(self, request, fp, code, msg, headers, newurl):
        from urllib.parse import urlsplit
        parsed = urlsplit(newurl)
        if (parsed.scheme != 'https' or parsed.port not in (None, 443)
                or parsed.hostname not in self.hosts):
            raise DependencyError('DEPENDENCY_NETWORK_ERROR')
        return super().redirect_request(request, fp, code, msg, headers, newurl)


def copy_binary(source, destination, limit=256 * 1024 * 1024):
    size = 0
    with open(destination, 'wb') as output:
        while chunk := source.read(1024 * 1024):
            size += len(chunk)
            if size > limit:
                raise DependencyError('DEPENDENCY_INVALID_ARCHIVE')
            output.write(chunk)


def install(tool, staging, emit, variant=None):
    if tool not in ('ffmpeg', 'javascript', transcription.MODEL_ID):
        raise DependencyError('ENGINE_PROTOCOL_ERROR')
    if variant not in (None, 'cpu', 'cuda') or (tool != transcription.MODEL_ID and variant is not None):
        raise DependencyError('ENGINE_PROTOCOL_ERROR')
    variant = variant or 'cpu'
    device = None
    if tool == transcription.MODEL_ID and variant == 'cuda':
        try:
            device = nano_cuda.require_device()
        except transcription.TranscriptionError as error:
            raise DependencyError(str(error)) from None
        if not transcription.vad_program().is_file():
            raise DependencyError('TRANSCRIPTION_ENGINE_UNAVAILABLE')
    current = status()
    if tool != transcription.MODEL_ID and next(row for row in current['tools'] if row['id'] == tool)['available']:
        return current
    stage = Path(staging)
    root = local_tools.MANAGED_DIR
    if not stage.is_dir() or stage.is_symlink() or stage.parent != root or any(stage.iterdir()):
        raise DependencyError('IO_ERROR')
    prepared = stage / 'prepared'
    prepared.mkdir()
    downloads = assets(tool, platform_key())
    target_name = 'fun-asr-nano-cuda' if tool == transcription.MODEL_ID and variant == 'cuda' else tool
    switching = tool == transcription.MODEL_ID and variant != nano_cuda.selected()
    existing = nano_cuda.directory() if variant == 'cuda' else root / transcription.MODEL_ID
    reuse_existing = switching and (nano_cuda.available() if variant == 'cuda' else transcription.model_available(existing))
    if tool == transcription.MODEL_ID and variant == 'cuda':
        if platform_key() != 'win32-x64':
            raise DependencyError('CUDA_UNSUPPORTED_SYSTEM')
        downloads = nano_cuda.downloads() + downloads[-1:]
    if reuse_existing:
        expected = nano_cuda.downloads()[2:] + assets(tool, platform_key())[-1:] if variant == 'cuda' else downloads
        for _, name, _, size, digest in expected:
            path = existing / name
            with path.open('rb') as stream:
                if path.stat().st_size != size or hashlib.file_digest(stream, 'sha256').hexdigest() != digest:
                    raise DependencyError('DEPENDENCY_CHECKSUM_MISMATCH')
        shutil.copytree(existing, prepared, dirs_exist_ok=True)
        downloads = []
    elif tool == transcription.MODEL_ID and variant == 'cuda':
        vad_asset = downloads[-1]
        for candidate in (root / transcription.MODEL_ID / vad_asset[1], nano_cuda.directory() / vad_asset[1]):
            if candidate.is_file() and not candidate.is_symlink():
                with candidate.open('rb') as stream:
                    if candidate.stat().st_size == vad_asset[3] and hashlib.file_digest(stream, 'sha256').hexdigest() == vad_asset[4]:
                        shutil.copyfile(candidate, prepared / vad_asset[1]); downloads.pop(); break
    if not downloads and not reuse_existing:
        raise DependencyError('DEPENDENCY_UNSUPPORTED_SYSTEM')
    # Use the system's proxy, never a platform Cookie or credential snapshot.
    context = ssl.create_default_context(cafile=certifi.where())
    total = sum(item[3] for item in downloads)
    received, last = 0, 0
    deadline = time.monotonic() + (1800 if tool == transcription.MODEL_ID else 300)

    def progress(phase):
        emit('dependency_progress', tool=tool, phase=phase, downloaded_bytes=received, total_bytes=total)

    try:
        progress('downloading' if downloads else 'installing')
        for index, (url, filename, compression, size, digest) in enumerate(downloads):
            archive = stage / f'{index}.part'
            checksum, count = hashlib.sha256(), 0
            request = Request(url, headers={'User-Agent': 'Yingqu-runtime/1', 'Accept-Encoding': 'identity'})
            opener = build_opener(HTTPSHandler(context=context), DownloadRedirects(model=url.startswith('https://huggingface.co/')))
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
            elif compression in ('nano-native', 'nano-cudart'):
                names = nano_cuda.NATIVE_FILES if compression == 'nano-native' else nano_cuda.CUDA_FILES
                with zipfile.ZipFile(archive) as package:
                    for name in names:
                        with package.open(name) as source:
                            copy_binary(source, prepared / name, limit=1536 * 1024 * 1024)
            else:
                shutil.copyfile(archive, target)
        binaries = [] if tool == transcription.MODEL_ID else ['ffmpeg', 'ffprobe'] if tool == 'ffmpeg' else ['deno']
        progress('installing')
        if tool == transcription.MODEL_ID:
            if variant == 'cuda':
                transcription.smoke(prepared, stage, lambda: progress('installing'), 'cuda', device)
                shutil.copyfile(transcription.resources() / 'NVIDIA-CUDA-LICENSE', prepared / 'NVIDIA-CUDA-LICENSE')
                shutil.copyfile(transcription.resources() / 'models-LICENSE', prepared / 'models-LICENSE')
                manifest = dict(nano_cuda.MANIFEST_ID, files={name: (prepared / name).stat().st_size for name in
                                nano_cuda.NATIVE_FILES + nano_cuda.CUDA_FILES + (nano_cuda.MODEL_FILE, nano_cuda.VAD_FILE, 'models-LICENSE', 'NVIDIA-CUDA-LICENSE')})
            else:
                transcription.smoke(prepared, stage, lambda: progress('installing'))
                shutil.copyfile(transcription.resources() / 'models-LICENSE', prepared / 'LICENSE')
                manifest = transcription.MANIFEST
            (prepared / 'manifest.json').write_text(json.dumps(manifest), encoding='utf-8')
        for name in binaries:
            file = prepared / (name + ('.exe' if sys.platform == 'win32' else ''))
            file.chmod(0o755)
            if sys.platform == 'darwin':
                signed = subprocess.run(['/usr/bin/codesign', '--force', '--sign', '-', str(file)],
                                        capture_output=True, timeout=15)
                if signed.returncode:
                    raise DependencyError('DEPENDENCY_VALIDATION_FAILED')
            output = local_tools.tool_output(file, '--version' if name == 'deno' else '-version')
            pattern = (r'^deno 2\.9\.7(?:\s|$)' if name == 'deno' else
                       rf'^{name} version {re.escape(FFMPEG_VERSIONS[platform_key()])}(?:\s|$|-)')
            if not re.match(pattern, output):
                raise DependencyError('DEPENDENCY_VALIDATION_FAILED')
        if not reuse_existing:
            (prepared / 'SOURCE.txt').write_text('\n'.join(url for url, *_ in downloads) + '\n', encoding='utf-8')
        committed = []
        backups = stage / 'previous'
        backups.mkdir()
        try:
            for name in [target_name]:
                source = prepared
                target = root / name
                if target.exists(): target.rename(backups / name)
                committed.append(name)
                source.rename(target)
            if tool == transcription.MODEL_ID:
                selection = nano_cuda.selection_file()
                if selection.exists():
                    shutil.copyfile(selection, backups / selection.name)
                (stage / 'selection.json').write_text(json.dumps(dict(variant=variant)), encoding='utf-8')
                os.replace(stage / 'selection.json', selection)
                committed.append(selection.name)
            result = status()
            if not next(row for row in result['tools'] if row['id'] == tool)['available']:
                raise DependencyError('DEPENDENCY_VALIDATION_FAILED')
        except (OSError, DependencyError):
            rejected = stage / 'rejected'
            rejected.mkdir()
            for name in reversed(committed):
                target = root / name
                if target.exists(): target.rename(rejected / name)
                if (backups / name).exists(): (backups / name).rename(target)
            raise
        return result
    except transcription.TranscriptionError as error:
        raise DependencyError(str(error)) from None
    except (TimeoutError, subprocess.TimeoutExpired):
        raise DependencyError('DEPENDENCY_TIMEOUT') from None
    except URLError:
        raise DependencyError('DEPENDENCY_NETWORK_ERROR') from None
    except (gzip.BadGzipFile, zipfile.BadZipFile, KeyError, EOFError):
        raise DependencyError('DEPENDENCY_INVALID_ARCHIVE') from None
