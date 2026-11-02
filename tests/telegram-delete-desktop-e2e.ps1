param(
    [Parameter(Mandatory=$true)] [int]$AppProcessId,
    [Parameter(Mandatory=$true)] [string]$FixtureManifest,
    [Parameter(Mandatory=$true)] [string]$Observations,
    [Parameter(Mandatory=$true)] [string]$ContextMenuHelper,
    [switch]$Resume
)
# Run with PowerShell on Windows. The isolated compile also includes the QA-only
# observer and its private manifest. Never fall back to another top-level window.
$ErrorActionPreference='Stop'
Add-Type -AssemblyName UIAutomationClient,UIAutomationTypes
$fixtures=@(Get-Content -Raw -Encoding UTF8 -LiteralPath $FixtureManifest | ConvertFrom-Json)
if ($fixtures.Count -ne 20 -or @($fixtures.assetId|Select-Object -Unique).Count -ne 20) { throw 'Expected 20 private fixtures' }
if ((Test-Path -LiteralPath $Observations) -ne [bool]$Resume) {throw 'An existing report requires explicit resume'}
function Find-Node($property,$value,$seconds=10,$type=$null) {
    $condition=New-Object System.Windows.Automation.PropertyCondition($property,$value)
    if ($type) {
        $byType=[System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::ControlTypeProperty,$type)
        $condition=[System.Windows.Automation.AndCondition]::new($condition,$byType)
    }
    $deadline=[DateTime]::UtcNow.AddSeconds($seconds)
    do {
        $found=$script:window.FindFirst([System.Windows.Automation.TreeScope]::Descendants,$condition)
        if ($found) { return $found }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Missing UI Automation element $value"
}
$proc=Get-Process -Id $AppProcessId
$handle=$proc.MainWindowHandle
if ($handle -eq [IntPtr]::Zero -or $proc.MainWindowTitle -ne 'Telegram delete E2E QA 1102b') {throw 'Expected isolated QA main window'}

$window=[System.Windows.Automation.AutomationElement]::FromHandle($handle)
$id=[System.Windows.Automation.AutomationElement]::AutomationIdProperty
$name=[System.Windows.Automation.AutomationElement]::NameProperty
$reporter=Find-Node $id 'qa-delete-observations' 45
$report=$reporter.GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern)
if ($Resume) {
    $saved=Get-Content -Raw -Encoding UTF8 -LiteralPath $Observations|ConvertFrom-Json
    $current=$report.Current.Value|ConvertFrom-Json
    foreach ($entry in $saved.samples.PSObject.Properties) {
        $key=$entry.Name
        if ($current.samples.$key.click.epoch -ne $entry.Value.click.epoch) {throw 'Cannot resume a different WebView or run'}
    }
}
function Save-Observed {
    $field=Find-Node $id 'qa-delete-observations'
    $raw=$field.GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern).Current.Value
    if (!$raw) {return}
    [IO.File]::WriteAllText($Observations,$raw)
    return ($raw|ConvertFrom-Json)
}
$deadline=[DateTime]::UtcNow.AddMinutes(61)
do {
    $data=Save-Observed
    if ($data.state -in @('ready','mounted')) {break}
    if ($data.state -and $data.state -notin @('starting','uploading')) {throw $data.state}
    Start-Sleep -Seconds 1
} while ([DateTime]::UtcNow -lt $deadline)
if ($data.state -eq 'ready') {
    $nav=Find-Node $name '下载任务' 10 ([System.Windows.Automation.ControlType]::Button)
    $nav.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke()
}
$deadline=[DateTime]::UtcNow.AddSeconds(45)
do {$data=Save-Observed; if($data.state -eq 'mounted'){break}; Start-Sleep -Milliseconds 200} while ([DateTime]::UtcNow -lt $deadline)
if ($data.state -ne 'mounted') {throw 'All 20 run-owned rows must be mounted before the first click'}
foreach($fixture in $fixtures) {
    $key=[guid]::Parse($fixture.assetId).ToString()
    if ($data.samples.$key.click) {continue}
    & $ContextMenuHelper $handle.ToInt64() $key
    if ($LASTEXITCODE) {throw "Could not open run-owned context menu"}
    $item=Find-Node $name '从当前同步库删除' 10 ([System.Windows.Automation.ControlType]::MenuItem)
    $item.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke()
    $deadline=[DateTime]::UtcNow.AddSeconds(15)
    do {$data=Save-Observed; if ($data.samples.$key.click) {break}; Start-Sleep -Milliseconds 50} while ([DateTime]::UtcNow -lt $deadline)
    if (!$data.samples.$key.click) {throw 'Expected trusted click for this exact fixture'}
}
$deadline=[DateTime]::UtcNow.AddMinutes(15)
do {
    $data=Save-Observed
    $done=@($fixtures|Where-Object {$key=$_.assetId;$data.samples.$key.frame}).Count
    if ($done -eq 20) {break}
    Start-Sleep -Seconds 1
} while ([DateTime]::UtcNow -lt $deadline)
if ($done -ne 20) {throw "Only $done rows reached their removal frame"}
'20 trusted native menu clicks and WebView row removals'
