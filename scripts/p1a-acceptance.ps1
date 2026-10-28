param(
    [ValidateSet('empty', 'upgrade', 'restore')][string]$Scenario = 'empty',
    [ValidatePattern('^[a-z0-9-]{1,48}$')][string]$RunId = (Get-Date -Format 'yyyyMMdd-HHmmss'),
    [switch]$BuildOnly
)
$ErrorActionPreference = 'Stop'
$workspace = Split-Path -Parent $PSScriptRoot
$identifier = "me.disign.yingqu.p1a-$RunId-$Scenario"
$data = Join-Path $env:APPDATA $identifier
$output = Join-Path $workspace ".local\p1a-acceptance\$RunId\$Scenario"
$marker = Join-Path $data 'p1a-acceptance.json'
if ((Test-Path -LiteralPath $data) -and !(Test-Path -LiteralPath $marker)) {
    throw "Refusing unmarked profile: $data"
}
if (Test-Path -LiteralPath $marker) {
    $existing = Get-Content -LiteralPath $marker -Raw | ConvertFrom-Json
    if ($existing.identifier -ne $identifier -or $existing.workspace -ne $workspace -or $existing.scenario -ne $Scenario) {
        throw "Profile marker does not match this run: $data"
    }
}
New-Item -ItemType Directory -Force -Path $data, $output | Out-Null
$markerJson = @{ identifier = $identifier; workspace = $workspace; scenario = $Scenario } | ConvertTo-Json
[System.IO.File]::WriteAllText($marker, $markerJson, [System.Text.UTF8Encoding]::new($false))
& (Join-Path $workspace '.venv\Scripts\python.exe') -X utf8 (Join-Path $workspace 'tests\fixtures\seed_p1a_profile.py') --data $data --output $output --scenario $Scenario
if ($LASTEXITCODE -ne 0) { throw 'Cannot prepare isolated profile' }
$config = @{identifier=$identifier; app=@{windows=@(@{label='main';title="映取 P1A $Scenario 隔离验收";width=1440;height=900;minWidth=1024;minHeight=680;decorations=$true;theme='Light';backgroundColor='#f9f9fb'})}} | ConvertTo-Json -Depth 6 -Compress
$previousConfig = $env:TAURI_CONFIG
try {
    $env:TAURI_CONFIG = $config
    & cargo build --locked --manifest-path (Join-Path $workspace 'src-tauri\Cargo.toml') --features desktop --example p1a-acceptance
    if ($LASTEXITCODE -ne 0) { throw 'Acceptance build failed' }
} finally {
    $env:TAURI_CONFIG = $previousConfig
}
$program = Join-Path $output 'p1a-acceptance.exe'
Copy-Item -LiteralPath (Join-Path $workspace 'src-tauri\target\debug\examples\p1a-acceptance.exe') -Destination $program
@{ identifier=$identifier; data=$data; output=$output; program=$program } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $output 'profile.json') -Encoding utf8
if (!$BuildOnly) {
    Start-Process -FilePath $program -WorkingDirectory $workspace -WindowStyle Hidden -RedirectStandardError (Join-Path $output 'stderr.log') -RedirectStandardOutput (Join-Path $output 'stdout.log')
}
Write-Output "Profile: $data"
Write-Output "Evidence: $output"
