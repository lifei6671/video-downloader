"""Pinned audio.cpp Windows CUDA package; independent FSMN-VAD stays on CPU."""
import ctypes
import json
import os
import platform
import sys
import uuid
from pathlib import Path

import local_tools

RELEASE = 'v0.9.2-drama3-base-small'
MODEL_REVISION = 'ce72677f84900f0dc57f498ace253bfb3c9155b6'
PIPELINE_REVISION = 'fun-asr-nano-audiocpp-50749561-fsmn-v1'
MODEL_FILE = 'fun-asr-nano-2512-q8_0.gguf'
MODEL_SIZE = 1045334432
VAD_FILE = 'fsmn-vad.gguf'
VAD_SIZE = 1720512
# Whitelists intentionally exclude the server, manager, converters and other models.
NATIVE_FILES = ('audiocpp_cli.exe', 'ggml.dll', 'ggml-base.dll', 'ggml-cuda.dll',
                'ggml-cpu-x64.dll', 'msvcp140.dll', 'msvcp140_1.dll', 'msvcp140_2.dll',
                'msvcp140_atomic_wait.dll', 'msvcp140_codecvt_ids.dll', 'vcruntime140.dll',
                'vcruntime140_1.dll', 'vcruntime140_threads.dll', 'LICENSE')
CUDA_FILES = ('cublasLt64_12.dll', 'cublas64_12.dll', 'cudart64_12.dll', 'cufft64_11.dll')
MANIFEST_ID = dict(release=RELEASE, model_revision=MODEL_REVISION, pipeline_revision=PIPELINE_REVISION)


def downloads():
    base = f'https://github.com/0xShug0/audio.cpp/releases/download/{RELEASE}/audio-{RELEASE}'
    return [
        (base + '-bin-windows-x64-cuda12.4.zip', 'native.zip', 'nano-native', 464268606,
         '48852da40790c40f3285bca6c53ff7829b3963080f0e95b6742f868fc1d7c757'),
        (base + '-cudart-windows-x64-cuda12.4.zip', 'cuda.zip', 'nano-cudart', 607273675,
         '4b1186a525c9ac70a4fbd031b47e0dfca2ebaa36cfcb760d5be1a76eb2b12da5'),
        (f'https://huggingface.co/FunAudioLLM/Fun-ASR-Nano-2512-GGUF/resolve/{MODEL_REVISION}/{MODEL_FILE}',
         MODEL_FILE, 'raw', MODEL_SIZE, '4d727357574b079b7f43336b2930f39da086ca02f5d8d50872090b4c1c3d5e0a')]


def directory():
    return local_tools.MANAGED_DIR / 'fun-asr-nano-cuda'


def selection_file():
    return local_tools.MANAGED_DIR / 'fun-asr-nano-backend.json'


def selected():
    try:
        value = json.loads(selection_file().read_text('utf-8'))
    except FileNotFoundError:
        return 'cpu'
    except (OSError, ValueError):
        return None
    return value.get('variant') if isinstance(value, dict) and value.get('variant') in ('cpu', 'cuda') else None


def available(root=None):
    root = Path(root) if root is not None else directory()
    try:
        manifest = json.loads((root / 'manifest.json').read_text('utf-8'))
        expected = set(NATIVE_FILES + CUDA_FILES + (MODEL_FILE, VAD_FILE, 'models-LICENSE', 'NVIDIA-CUDA-LICENSE'))
        return (not root.is_symlink() and all(manifest.get(k) == v for k, v in MANIFEST_ID.items())
                and set(manifest['files']) == expected
                and manifest['files'][MODEL_FILE] == MODEL_SIZE and manifest['files'][VAD_FILE] == VAD_SIZE
                and all(type(size) is int and size > 0 and not (root / name).is_symlink()
                        and (root / name).is_file() and (root / name).stat().st_size == size
                        for name, size in manifest['files'].items()))
    except (OSError, ValueError, KeyError, TypeError, AttributeError):
        return False


def devices():
    """Only called for explicit CUDA install or CUDA inference, never ordinary detection."""
    driver = ctypes.WinDLL(str(Path(os.environ['SystemRoot']) / 'System32/nvcuda.dll'))
    def call(name, *args):
        if getattr(driver, name)(*args) != 0:
            raise OSError('CUDA driver query failed')
    call('cuInit', 0)
    version, count = ctypes.c_int(), ctypes.c_int()
    call('cuDriverGetVersion', ctypes.byref(version))
    call('cuDeviceGetCount', ctypes.byref(count))
    result = []
    for ordinal in range(count.value):
        device, major, minor = ctypes.c_int(), ctypes.c_int(), ctypes.c_int()
        name, identifier = ctypes.create_string_buffer(256), ctypes.create_string_buffer(16)
        call('cuDeviceGet', ctypes.byref(device), ordinal)
        call('cuDeviceGetName', name, len(name), device)
        call('cuDeviceComputeCapability', ctypes.byref(major), ctypes.byref(minor), device)
        call('cuDeviceGetUuid', identifier, device)
        result.append(dict(name=name.value.decode('utf-8', errors='replace'), driver=version.value,
                           compute=f'{major.value}.{minor.value}',
                           device='GPU-' + str(uuid.UUID(bytes=identifier.raw))))
    return result


def detect():
    if sys.platform != 'win32' or platform.machine().lower() not in ('amd64', 'x86_64'):
        return dict(reason='unsupported_system')
    try:
        found = devices()
    except (OSError, AttributeError, KeyError):
        return dict(reason='driver_unavailable')
    # Deliberate tested package boundary; do not assume a CUDA 12.4 binary supports Blackwell.
    supported = {'7.5', '8.0', '8.6', '8.9', '9.0'}
    selected_device = next((g for g in found if g['compute'] in supported and g['driver'] >= 12040), None)
    if selected_device:
        return dict(selected_device, reason=None)
    candidate = next((g for g in found if g['compute'] in supported), None)
    return dict(candidate or {}, reason='driver_too_old' if candidate else 'unsupported_gpu' if found else 'driver_unavailable')


def require_device():
    from transcription import TranscriptionError
    gpu = detect()
    if gpu['reason']:
        raise TranscriptionError({'unsupported_system': 'CUDA_UNSUPPORTED_SYSTEM',
                                  'driver_unavailable': 'CUDA_DRIVER_UNAVAILABLE',
                                  'driver_too_old': 'CUDA_DRIVER_TOO_OLD',
                                  'unsupported_gpu': 'CUDA_UNSUPPORTED_GPU'}[gpu['reason']])
    return gpu['device']


def environment(root, device):
    env = os.environ.copy()
    # PyInstaller's DLL search directory belongs to the sidecar, not this foreign executable.
    env['PATH'] = str(root) + os.pathsep + env.get('PATH', '')
    env['CUDA_VISIBLE_DEVICES'] = device
    env['VAD_MAXSEG'] = '30000'
    return env
