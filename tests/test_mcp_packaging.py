"""Installation checks must reject incomplete or mismatched shipped files."""
import importlib.util
import shutil
import subprocess
import textwrap
from pathlib import Path

import pytest


ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location('check_packaged_mcp', ROOT / 'scripts/check_packaged_mcp.py')
packaging = importlib.util.module_from_spec(spec)
spec.loader.exec_module(packaging)


def test_missing_helper_is_not_a_valid_install(tmp_path):
    with pytest.raises(ValueError, match='Missing packaged MCP file'):
        packaging.check_app(tmp_path)


def test_mismatched_notices_are_rejected_before_launch(tmp_path):
    for name in ['yingqu.exe', 'yingqu-mcp.exe', 'yingqu', 'yingqu-mcp']:
        (tmp_path / name).write_text('not executable')
    (tmp_path / 'licenses').mkdir()
    (tmp_path / 'licenses/mcp-licenses.txt').write_text('outdated or incomplete notices')
    with pytest.raises(ValueError, match='licenses do not match'):
        packaging.check_app(tmp_path)


def test_nsis_comparison_allows_only_the_expected_main_marker(tmp_path):
    built, installed = tmp_path / 'build', tmp_path / 'installed'
    built.mkdir(); installed.mkdir()
    (built / 'yingqu-mcp.exe').write_bytes(b'native helper')
    (installed / 'yingqu-mcp.exe').write_bytes(b'native helper')
    (built / 'yingqu.exe').write_bytes(b'prefix__TAURI_BUNDLE_TYPE_VAR_UNKsuffix')
    (installed / 'yingqu.exe').write_bytes(b'prefix__TAURI_BUNDLE_TYPE_VAR_NSSsuffix')
    packaging.verify_windows_binaries(installed, built, 'yingqu.exe')
    (installed / 'yingqu.exe').write_bytes(b'prefix__TAURI_BUNDLE_TYPE_VAR_NSSchanged')
    with pytest.raises(ValueError, match='beyond the Tauri NSIS marker'):
        packaging.verify_windows_binaries(installed, built, 'yingqu.exe')
    (installed / 'yingqu-mcp.exe').write_bytes(b'old helper')
    with pytest.raises(ValueError, match='MCP helper differs'):
        packaging.verify_windows_binaries(installed, built, 'yingqu.exe')


@pytest.mark.parametrize('workflow,step', [('ci.yml', 'Build desktop'), ('release.yml', 'Build installer')])
@pytest.mark.parametrize('signing_code', [0, 42])
def test_build_checks_preserve_signing_failures_and_fill_cache(tmp_path, workflow, step, signing_code):
    pwsh = shutil.which('pwsh')
    if not pwsh:
        pytest.skip('PowerShell 7 is required for the CI shell failure-path check')
    lines = (ROOT / '.github/workflows' / workflow).read_text(encoding='utf-8').splitlines()
    start = lines.index(f'      - name: {step}')
    start = next(i + 1 for i in range(start, len(lines)) if lines[i] == '        run: |')
    end = next((i for i in range(start, len(lines)) if lines[i] and not lines[i].startswith('          ')), len(lines))
    commands = textwrap.dedent('\n'.join(lines[start:end]))
    # Exercise the macOS shell branch on this host, substituting only external
    # commands. A signing error must survive a subsequent successful Python check.
    commands = commands.replace('$IsMacOS', '$true').replace('$IsWindows', '$false')
    commands = commands.replace('${{ matrix.bundles }}', 'dmg')
    script = tmp_path / 'ci-failure.ps1'
    script.write_text("""$ErrorActionPreference = 'Stop'
$global:BuiltDesktop = $false
function npm { $global:BuiltDesktop = $true; $global:LASTEXITCODE = 0 }
function python {
    if ($args[0] -eq 'scripts/update_mcp_licenses.py' -and !$global:BuiltDesktop) {
        $global:LASTEXITCODE = 37
    } else { $global:LASTEXITCODE = 0 }
}
""" + f'function codesign {{ $global:LASTEXITCODE = {signing_code} }}\n'
        + commands + '\nexit $LASTEXITCODE\n', encoding='utf-8-sig')
    result = subprocess.run([pwsh, '-NoProfile', '-File', str(script)], capture_output=True, timeout=15)
    assert result.returncode == signing_code, result.stderr.decode(errors='replace')
