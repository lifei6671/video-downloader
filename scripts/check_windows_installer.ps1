param(
    [Parameter(Mandatory = $true)][string]$Installer,
    [ValidatePattern('^[a-z0-9-]{1,48}$')][string]$RunId = 'check',
    [ValidatePattern('^[a-zA-Z0-9_-]+\.exe$')][string]$MainBinary = 'yingqu.exe',
    [switch]$KeepInstalled
)
$ErrorActionPreference = 'Stop'
if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) { throw 'Windows NSIS verification requires Windows' }
$workspace = Split-Path -Parent $PSScriptRoot
$installerPath = (Resolve-Path -LiteralPath $Installer).Path
$output = Join-Path $workspace ".local\wb207-install\$RunId"
$destination = [IO.Path]::GetFullPath((Join-Path $output '安装 Program Files'))
$allowed = [IO.Path]::GetFullPath($output) + [IO.Path]::DirectorySeparatorChar
if (!$destination.StartsWith($allowed, [StringComparison]::OrdinalIgnoreCase)) { throw 'Unexpected installation directory' }
if (Test-Path -LiteralPath $destination) { throw "Refusing to replace an existing installation: $destination" }
New-Item -ItemType Directory -Force -Path $output | Out-Null
$record = @{ workspace = $workspace; installer = $installerPath; installer_sha256 = (Get-FileHash -LiteralPath $installerPath -Algorithm SHA256).Hash; destination = $destination; main_binary = $MainBinary }
$record | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $output 'installation.json') -Encoding utf8
$verified = $false
try {
    # NSIS requires /D last, with the unquoted remainder as its literal directory.
    $process = Start-Process -FilePath $installerPath -ArgumentList @('/S', "/D=$destination") -WindowStyle Hidden -PassThru -Wait
    if ($process.ExitCode -ne 0) { throw "NSIS installation failed: $($process.ExitCode)" }
    $python = Join-Path $workspace ('.venv\Scripts\python.exe')
    & $python -X utf8 (Join-Path $PSScriptRoot 'check_packaged_mcp.py') $destination --main-binary $MainBinary --build-dir (Join-Path $workspace 'src-tauri\target\release')
    if ($LASTEXITCODE -ne 0) { throw 'Installed MCP check failed' }
    $hashes = @{}
    foreach ($binary in @($MainBinary, 'yingqu-mcp.exe')) {
        $installedHash = (Get-FileHash -LiteralPath (Join-Path $destination $binary) -Algorithm SHA256).Hash
        $hashes[$binary] = $installedHash
    }
    $record['binaries'] = $hashes
    $record['comparison'] = 'helper identical; main identical after the Tauri NSIS marker patch only'
    $record['status'] = 'PASSED'
    $record | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $output 'installation.json') -Encoding utf8
    $verified = $true
    Write-Output "Installed MCP verified: $destination"
} finally {
    if (!$KeepInstalled -or !$verified) {
        $uninstaller = Join-Path $destination 'uninstall.exe'
        if (Test-Path -LiteralPath $uninstaller) {
            $process = Start-Process -FilePath $uninstaller -ArgumentList @('/S', "_?=$destination") -WindowStyle Hidden -PassThru -Wait
            if ($process.ExitCode -ne 0) { throw "NSIS uninstall failed: $($process.ExitCode)" }
        }
    }
}
