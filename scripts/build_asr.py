"""Build the pinned, CPU-only Fun-ASR-Nano helper for the current host architecture."""
import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys
import tarfile
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
FUNASR = 'fbbfe01800a8c4c9103cb2150fb1a5d06cc64758'
LLAMA = '803b7fcae893e9caaee3921779628fef83ac0965'


def instrument_progress(runtime):
    """Add flushed counters to the pinned CLI, without changing inference or stdout."""
    source = runtime / 'fun-asr-nano/funasr-cli/funasr-cli.cpp'
    text = source.read_text('utf-8')
    edits = [
        ('        std::vector<std::pair<int,int>> segs; // ms',
         '        fprintf(stderr,"YINGQU_ASR_PROGRESS segmenting 0 0\\n"); fflush(stderr);\n'
         '        std::vector<std::pair<int,int>> segs; // ms'),
        ('    int srt_idx = 0;',
         '    int progress_total = 0, progress_done = 0;\n'
         '    for (const auto& w : wins) if (w.second >= WINLEN) ++progress_total;\n'
         '    fprintf(stderr,"YINGQU_ASR_PROGRESS recognizing 0 %d\\n",progress_total); fflush(stderr);\n'
         '    int srt_idx = 0;'),
        ('        fflush(stdout);',
         '        fflush(stdout);\n'
         '        fprintf(stderr,"YINGQU_ASR_PROGRESS recognizing %d %d\\n",++progress_done,progress_total); fflush(stderr);'),
    ]
    for before, after in edits:
        if text.count(after) == 1:
            continue
        if text.count(before) != 1:
            raise RuntimeError('Pinned Fun-ASR progress instrumentation no longer applies')
        text = text.replace(before, after, 1)
    source.write_text(text, encoding='utf-8', newline='\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cmake', default='cmake')
    args = parser.parse_args()
    work = ROOT / '.local/sensevoice-build'
    work.mkdir(parents=True, exist_ok=True)
    sources = {}
    for repo, revision in [('modelscope/FunASR', FUNASR), ('ggml-org/llama.cpp', LLAMA)]:
        name = repo.split('/')[-1]
        archive = work / (name + '.tar.gz')
        if not archive.exists():
            with urllib.request.urlopen(f'https://codeload.github.com/{repo}/tar.gz/{revision}', timeout=60) as response, archive.open('wb') as out:
                shutil.copyfileobj(response, out)
        with archive.open('rb') as stream:
            digest = hashlib.file_digest(stream, 'sha256').hexdigest()
        expected = {'FunASR': 'be42f6f6fa86468c3db8ea93b7c1ae80e7389a49be9df989f82a5c59577290bd',
                    'llama.cpp': '8fe8528e89d8fca8c8b81696efa83ff82c996bf84d17be65e884c1ceae35351e'}[name]
        if digest != expected:
            raise RuntimeError(f'{name}: source archive SHA-256 mismatch')
        source = work / f'{name}-{revision}'
        if not source.exists():
            with tarfile.open(archive) as package:
                package.extractall(work, filter='data')
        sources[name] = source
        print(f'{repo} {revision} sha256={digest}', flush=True)
    runtime = sources['FunASR'] / 'runtime/llama.cpp'
    instrument_progress(runtime)
    build = work / 'build'
    command = [args.cmake, '-S', str(runtime), '-B', str(build),
               '-DCMAKE_BUILD_TYPE=Release', '-DGGML_NATIVE=OFF', '-DGGML_AVX=OFF',
               '-DGGML_AVX2=OFF', '-DGGML_FMA=OFF', '-DGGML_F16C=OFF',
               '-DGGML_CUDA=OFF', '-DGGML_VULKAN=OFF', '-DGGML_METAL=OFF',
               '-DGGML_OPENMP=OFF', '-DGGML_LLAMAFILE=OFF', '-DGGML_SSE42=OFF', '-DGGML_BMI2=OFF',
               f'-DFETCHCONTENT_SOURCE_DIR_LLAMA={sources["llama.cpp"]}']
    if sys.platform == 'win32':
        vswhere = Path(os.environ['ProgramFiles(x86)']) / 'Microsoft Visual Studio/Installer/vswhere.exe'
        version = subprocess.check_output([str(vswhere), '-latest', '-products', '*', '-property', 'installationVersion'], text=True).strip()
        generator = 'Visual Studio 18 2026' if version.startswith('18.') else 'Visual Studio 17 2022'
        command += ['-G', generator, '-A', 'x64', '-DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreaded', '-DCMAKE_CXX_FLAGS=/utf-8 /EHsc']
        command += [f'-DCMAKE_EXE_LINKER_FLAGS=/MANIFEST:EMBED /MANIFESTINPUT:"{ROOT / "vendor/asr-windows.manifest"}"']
    elif sys.platform == 'darwin':
        command += ['-DCMAKE_OSX_DEPLOYMENT_TARGET=12.0']
    subprocess.run(command, check=True)
    build_command = [args.cmake, '--build', str(build), '--config', 'Release', '--parallel',
                     '1' if sys.platform == 'win32' else '4', '--target', 'llama-funasr-cli', 'llama-funasr-vad']
    if sys.platform == 'win32':
        build_command += ['--', '/nr:false']
    subprocess.run(build_command, check=True)
    name = 'llama-funasr-cli' + ('.exe' if os.name == 'nt' else '')
    binaries = list((build / 'bin').rglob(name))
    if len(binaries) != 1:
        raise RuntimeError('Expected one native helper')
    output = ROOT / 'python-engine/asr'
    output.mkdir(exist_ok=True)
    for retired in ('llama-funasr-sensevoice', 'llama-funasr-sensevoice.exe'):
        (output / retired).unlink(missing_ok=True)
    shutil.copy2(binaries[0], output / name)
    (output / name).chmod(0o755)
    vad_name = 'llama-funasr-vad' + ('.exe' if os.name == 'nt' else '')
    vad_binaries = list((build / 'bin').rglob(vad_name))
    if len(vad_binaries) != 1:
        raise RuntimeError('Expected one standalone VAD helper')
    shutil.copy2(vad_binaries[0], output / vad_name)
    (output / vad_name).chmod(0o755)
    for label, source in sources.items():
        shutil.copy2(source / 'LICENSE', output / (label + '-LICENSE'))
    audio_source = (runtime / 'funasr-common/miniaudio.h').read_text('utf-8')
    license_start = audio_source.rfind('This software is available as a choice')
    if license_start < 0:
        raise RuntimeError('Missing miniaudio license')
    (output / 'miniaudio-LICENSE').write_text(audio_source[license_start:], encoding='utf-8')
    shutil.copy2(ROOT / 'python-engine/assets/sensevoice-smoke.wav', output / 'smoke.wav')
    shutil.copy2(ROOT / 'vendor/sensevoice-model-LICENSE', output / 'models-LICENSE')
    shutil.copy2(ROOT / 'vendor/nvidia-cuda-12.4-EULA.html', output / 'NVIDIA-CUDA-LICENSE')
    (output / 'SOURCE.json').write_text(json.dumps(dict(funasr=FUNASR, llama=LLAMA, backend='cpu', model='Fun-ASR-Nano', progress_version=1), indent=2) + '\n')
    if sys.platform == 'darwin':
        for executable in (name, vad_name):
            subprocess.run(['codesign', '--force', '--sign', '-', str(output / executable)], check=True)


if __name__ == '__main__':
    main()
