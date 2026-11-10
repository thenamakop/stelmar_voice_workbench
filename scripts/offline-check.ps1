<#
.SYNOPSIS
  Phase 0 gate "Shell opens with no network": starts the release build, times how long the window
  takes to appear and to render, checks that it is not blank, and saves screenshots and a log.

.DESCRIPTION
  Run it twice on the same machine: once with the network on (baseline) and once with Wi-Fi off
  and Ethernet unplugged. The second run compares its start-up times with the baseline log.

  For each run the script:
  1. stops any running copy of the app;
  2. starts target\release\stelmar-voice-workbench.exe;
  3. measures the time until the main window exists and until the page has rendered (the charcoal
     header is drawn and the window holds more than a blank fill);
  4. records any non-loopback TCP connection, non-loopback TCP listener or non-loopback UDP endpoint
     from the app's process tree and every msedgewebview2.exe for the app's user data folder;
  5. takes a screenshot (only when the app's window is in front) and stops the app.

  Verdict: offline, every run must render with no non-loopback socket (PASS or FAIL); online, the
  result is BASELINE, because the WebView2 runtime's own connections are expected there.

  At the end (unless -NoPrompt) it starts the app once more and asks you to click Record, then
  takes a screenshot of the Hindi, Urdu and mixed sample text for the font check.

  Whether the machine is online comes from scripts/lib/NetworkState.psm1: physical adapters,
  connection profiles and, when those say offline, a 2-second TCP probe to 1.1.1.1:443 and
  8.8.8.8:443 (no DNS, no HTTP). A default route alone never counts as online; routes on
  disconnected, virtual or pseudo-interfaces are listed as ignored, with the reason.

  No administrator rights or development tools needed. It never changes adapters or firewall rules.

  The log goes through scripts/lib/Redact.psm1 (the repository is public): the user profile path,
  user name and computer name are replaced; -Machine sets the name shown instead.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File .\scripts\offline-check.ps1 -Diagnose
  powershell -ExecutionPolicy Bypass -File .\scripts\offline-check.ps1 -Label online -Machine Legion
  powershell -ExecutionPolicy Bypass -File .\scripts\offline-check.ps1 -Label offline -Machine Legion
  (-OutDir defaults to docs\evidence\phase-<n> for the current phase branch, never phase-0 on a later phase.)
#>
[CmdletBinding()]
param(
  [ValidateSet('online', 'offline')] [string]$Label,
  [string]$Exe,
  [string]$OutDir,
  [int]$Runs = 3,
  [int]$TimeoutSeconds = 20,
  [switch]$NoPrompt,
  # Print the network state block and exit without starting the app.
  [switch]$Diagnose,
  # Machine name written to the log instead of the computer name (docs/GATES.md names, for example Legion).
  [string]$Machine = '%COMPUTERNAME%'
)
$ErrorActionPreference = 'Stop'
# Windows PowerShell 5.1 leaves $PSScriptRoot empty in param defaults, so resolve defaults here.
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
Import-Module (Join-Path $here 'lib\PsEnv.psm1') -Force
Set-WbCleanPSModulePath
if (-not $Exe) { $Exe = Join-Path $here '..\target\release\stelmar-voice-workbench.exe' }
# The evidence folder defaults to the current phase's (from the branch name phase-<n>-...), never to
# an earlier phase: an earlier phase's evidence must not be overwritten. Pass -OutDir to override.
if (-not $OutDir) {
  $branch = (& git -C $here rev-parse --abbrev-ref HEAD 2>$null)
  if ($branch -match '^phase-(\d+[a-z]?)-') { $OutDir = Join-Path $here ('..\docs\evidence\phase-' + $Matches[1]) }
  else { throw "Cannot tell the current phase from the branch name '$branch'. Pass -OutDir (for example docs\evidence\phase-1)." }
}
Import-Module (Join-Path $here 'lib\NetworkState.psm1') -Force
Import-Module (Join-Path $here 'lib\Redact.psm1') -Force

# Facts first; the probe runs only when the facts do not already show the machine online.
function Get-CheckedNetworkState {
  $facts = Get-NetworkFacts
  $result = Resolve-NetworkState -Facts $facts
  if ($result.State -ne 'Online') {
    $probes = @(Test-InternetProbe)
    $result = Resolve-NetworkState -Facts $facts -ProbeResults $probes
  }
  [pscustomobject]@{ Facts = $facts; Result = $result; Lines = @(Format-NetworkState -Facts $facts -Result $result) }
}

if ($Diagnose) {
  $net = Get-CheckedNetworkState
  $net.Lines | ForEach-Object { Write-Host ($_ | ConvertTo-WbRedacted -MachineLabel $Machine) }
  exit 0
}
if (-not $Label) { throw 'Give -Label online or -Label offline (or -Diagnose).' }

Import-Module (Join-Path $here 'lib\AppProcess.psm1') -Force

$Exe = (Resolve-Path $Exe).Path
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$OutDir = (Resolve-Path $OutDir).Path
$log = [System.Collections.Generic.List[string]]::new()
# The console gets the same redacted text as the log (user name, profile path, computer name).
function Say([string]$line) { Write-Host ($line | ConvertTo-WbRedacted -MachineLabel $Machine); $log.Add($line) }

# Clean stop between runs (AppProcess.psm1): close the window, force-kill what is left including every
# msedgewebview2.exe for the app's user data folder, and wait until none remain. A relaunch while the
# previous WebView2 is still exiting would measure that race instead of the app.
function Stop-App {
  if (-not (Stop-WbApp)) {
    Say "STOP: WebView2 from the previous run is still running (user data folder $(Get-WbUserDataDir))."
    Write-WbEvidence -Path (Join-Path $OutDir "offline-check-$Label.txt") -Lines $log -MachineLabel $Machine
    exit 3
  }
}

function Invoke-Run([int]$Index, [string]$ScreenshotName) {
  Stop-App
  $run = Start-WbApp -Exe $Exe -TimeoutSeconds $TimeoutSeconds
  $app = $run.Process; $handle = $run.Handle; $watch = $run.Watch; $remote = @{}; $final = $null
  # Watch connections for a few more seconds after render; WebView2 starts its own traffic at +3 to 4 s.
  # Processes watched: the app's tree plus every WebView2 process for the app's user data folder.
  # Recorded: TCP to a non-loopback address, TCP listening on a non-loopback address, and any UDP
  # endpoint bound to a non-loopback address (PLAN section 10).
  $loop = @('127.0.0.1', '::1')
  $until = $watch.Elapsed.TotalSeconds + 6
  while ($watch.Elapsed.TotalSeconds -lt $until -and -not $app.HasExited) {
    $ids = @((Get-WbProcessTree $app.Id).ProcessId) + @((Get-WbWebView2).ProcessId)
    foreach ($c in @(Get-NetTCPConnection -ErrorAction SilentlyContinue | Where-Object { $ids -contains $_.OwningProcess })) {
      if ($c.State -eq 'Listen') {
        if ($c.LocalAddress -notin $loop) { $remote["TCP listen $($c.LocalAddress):$($c.LocalPort)"] = $true }
      } elseif ($c.RemoteAddress -notin ($loop + @('0.0.0.0', '::'))) {
        $remote["$($c.RemoteAddress):$($c.RemotePort) $($c.State)"] = $true
      }
    }
    Get-NetUDPEndpoint -ErrorAction SilentlyContinue |
      Where-Object { $ids -contains $_.OwningProcess -and $_.LocalAddress -notin $loop } |
      ForEach-Object { $remote["UDP $($_.LocalAddress):$($_.LocalPort)"] = $true }
    Start-Sleep -Milliseconds 500
  }
  $alive = -not $app.HasExited
  $wv2 = @(Get-WbWebView2).Count
  if ($handle -ne [IntPtr]::Zero -and $alive) {
    $final = Get-WbWindowCheck $handle (Join-Path $OutDir $ScreenshotName)
  }
  $result = [pscustomobject]@{
    Run       = $Index
    WindowMs  = $run.WindowMs
    RenderMs  = $run.RenderMs
    Rendered  = [bool]($final -and $final.Rendered)
    Charcoal  = if ($final) { $final.Charcoal } else { $null }
    Colours   = if ($final) { $final.Colours } else { $null }
    Obscured  = [bool]($final -and $final.Obscured)
    Alive     = $alive
    WebView2  = $wv2
    Remote    = ($remote.Keys | Sort-Object) -join ', '
  }
  Stop-App
  $result
}

$logFile = Join-Path $OutDir "offline-check-$Label.txt"
$net = Get-CheckedNetworkState
Say "STELMAR Voice Workbench offline check: $Label"
Say "Date: $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss zzz')   Machine: $Machine   Windows $([Environment]::OSVersion.Version)"
Say "Exe: $Exe ($((Get-Item $Exe).LastWriteTime.ToString('yyyy-MM-dd HH:mm')))"
foreach ($line in $net.Lines) { Say $line }
if ($Label -eq 'offline' -and $net.Result.State -eq 'Online') {
  Say "STOP: the machine is online because:"
  foreach ($reason in $net.Result.Reasons) { Say "  - $reason" }
  Say "Turn Wi-Fi off and unplug Ethernet (and disconnect any VPN), then run again."
  Write-WbEvidence -Path $logFile -Lines $log -MachineLabel $Machine
  exit 2
}
if ($Label -eq 'offline' -and $net.Result.State -eq 'Unknown') {
  Say "WARNING: the network state could not be read fully; continuing because nothing shows the machine online."
}
if ($Label -eq 'online' -and $net.Result.State -ne 'Online') {
  Say "WARNING: the network state is $($net.Result.State); this baseline may not reflect a connected machine."
}

$results = @()
for ($i = 1; $i -le $Runs; $i++) {
  $r = Invoke-Run $i "shell-$Label-run$i.png"
  $results += $r
  $blank = if ($r.Obscured) { 'not checked: another window was in front, no screenshot taken' } else { "$($r.Rendered) (charcoal $($r.Charcoal)%, $($r.Colours) colours)" }
  Say ("Run {0}: window {1} ms, rendered {2} ms, not blank = {3}, app alive = {4}, WebView2 processes = {5}, non-loopback network = {6}" -f `
      $r.Run, $r.WindowMs, $r.RenderMs, $blank, $r.Alive, $r.WebView2, $(if ($r.Remote) { $r.Remote } else { 'none' }))
}

function Median($values) { $v = @($values | Where-Object { $null -ne $_ } | Sort-Object); if ($v.Count -eq 0) { return $null }; $v[[int][math]::Floor(($v.Count - 1) / 2)] }
$medWindow = Median ($results.WindowMs); $medRender = Median ($results.RenderMs)
Say "Median: window $medWindow ms, rendered $medRender ms (runs after the first are warm starts)"

$failed = @($results | Where-Object { -not $_.Rendered -or -not $_.Alive -or $null -eq $_.RenderMs })
# Offline, any non-loopback socket fails the gate. Online, connections are expected (the WebView2
# runtime's own TLS, a docs/GATES.md open item), so the online verdict is BASELINE, never PASS.
$withNetwork = @($results | Where-Object { $_.Remote })
if ($Label -eq 'offline' -and $withNetwork.Count) {
  Say "NETWORK: $($withNetwork.Count) run(s) opened non-loopback sockets with the network off."
  $failed += $withNetwork
}
if ($Label -eq 'offline') {
  $baselineFile = Join-Path $OutDir 'offline-check-online.txt'
  if (Test-Path $baselineFile) {
    $m = Select-String -Path $baselineFile -Pattern 'Median: window (\d+) ms, rendered (\d+) ms' | Select-Object -Last 1
    if ($m) {
      $bw = [int]$m.Matches[0].Groups[1].Value; $br = [int]$m.Matches[0].Groups[2].Value
      Say "Baseline (online): window $bw ms, rendered $br ms. Offline minus online: window $($medWindow - $bw) ms, rendered $($medRender - $br) ms"
      if ($null -ne $medRender -and $medRender -gt [math]::Max($br * 1.5, $br + 1000)) {
        Say "SLOW: offline render is more than 1.5x (and 1 s) slower than the online baseline."
        $failed += 'slow'
      }
    }
  } else {
    Say "No online baseline found ($baselineFile); run with -Label online first to compare."
  }
}

if (-not $NoPrompt) {
  Stop-App
  $app = Start-Process -FilePath $Exe -PassThru
  Write-Host ""
  Write-Host "The app is open. Click Record in the top bar, wait until the sample text shows, then press Enter here."
  [void](Read-Host)
  $app.Refresh()
  if ($app.MainWindowHandle -ne [IntPtr]::Zero) {
    $rec = Get-WbWindowCheck $app.MainWindowHandle (Join-Path $OutDir "record-$Label.png")
    if ($rec -and $rec.Obscured) { Say "Record screen screenshot: not taken, another window was in front of the app." }
    elseif ($rec) { Say "Record screen screenshot: record-$Label.png (not blank = $($rec.Rendered), $($rec.Colours) colours)" }
  }
  Stop-App
}

$verdict = if ($failed.Count) { 'FAIL' } elseif ($Label -eq 'online') { 'BASELINE' } else { 'PASS' }
if ($verdict -eq 'BASELINE') {
  Say "Result: BASELINE (all runs rendered; runs with non-loopback connections: $($withNetwork.Count) of $Runs, see the WebView2 open item in docs/GATES.md)"
} else {
  Say "Result: $verdict"
}
Write-WbEvidence -Path $logFile -Lines $log -MachineLabel $Machine
Write-Host "Log: $logFile"
if ($verdict -eq 'FAIL') { exit 1 }
