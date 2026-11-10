<#
.SYNOPSIS
  Phase 1 offline check with a recording: starts the release build, then watches every socket of
  the app's process tree and its WebView2 processes for several minutes while you record about a
  minute of audio. Any TCP connection or listener, and any UDP endpoint, on a non-loopback address
  is a FAIL (offline) or reported (online baseline).

.DESCRIPTION
  What you do while it runs: when the window shows, press Start on the Record screen (pick the
  microphone, leave the backup folder on a local drive), speak or play something for about a
  minute, press Stop, then wait for the script to finish. Do not touch the network settings
  while it runs.

  Run it twice with the same exe: once online (baseline: WebView2's own connections are expected)
  and once with Wi-Fi off and Ethernet unplugged (every non-loopback socket is a FAIL). The
  network state comes from scripts/lib/NetworkState.psm1, like scripts/offline-check.ps1.

  The log goes through scripts/lib/Redact.psm1 (the repository is public).

.EXAMPLE
  powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\offline-session-check.ps1 -Label offline -Machine Legion -OutDir .\docs\evidence\phase-1
#>
[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)] [ValidateSet('online', 'offline')] [string]$Label,
  [string]$Exe,
  [string]$OutDir,
  [int]$WatchSeconds = 240,
  [string]$Machine = '%COMPUTERNAME%'
)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
Import-Module (Join-Path $here 'lib\PsEnv.psm1') -Force
Set-WbCleanPSModulePath
if (-not $Exe) { $Exe = Join-Path $here '..\target\release\stelmar-voice-workbench.exe' }
if (-not $OutDir) { $OutDir = Join-Path $here '..\docs\evidence\phase-1' }
Import-Module (Join-Path $here 'lib\NetworkState.psm1') -Force
Import-Module (Join-Path $here 'lib\Redact.psm1') -Force
Import-Module (Join-Path $here 'lib\AppProcess.psm1') -Force

$log = New-Object System.Collections.Generic.List[string]
function Say([string]$line) { Write-Host ($line | ConvertTo-WbRedacted -MachineLabel $Machine); $log.Add($line) }

$exePath = (Resolve-Path -LiteralPath $Exe).Path
$hash = (Get-FileHash -LiteralPath $exePath -Algorithm SHA256).Hash.ToLower()
$gateStrings = 0
$bytes = [System.IO.File]::ReadAllBytes($exePath)
$text = [System.Text.Encoding]::ASCII.GetString($bytes)
$gateStrings = ([regex]::Matches($text, 'WB_GATE_')).Count

$facts = Get-NetworkFacts
$state = Resolve-NetworkState -Facts $facts
if ($state.State -ne 'Online') { $state = Resolve-NetworkState -Facts $facts -ProbeResults @(Test-InternetProbe) }
$expectOffline = ($Label -eq 'offline')
Say "Phase 1 offline check with a recording ($Label)"
Say "date: $((Get-Date).ToString('yyyy-MM-dd HH:mm:ss zzz'))"
Say "machine: $Machine"
Say "exe: $exePath"
Say "exe sha256: $hash ($((Get-Item -LiteralPath $exePath).Length) bytes, built $((Get-Item -LiteralPath $exePath).LastWriteTime.ToString('yyyy-MM-dd HH:mm')))"
Say "WB_GATE_ strings in the exe: $gateStrings (must be 0: no gate hooks in a release build)"
Say "watch window: $WatchSeconds s"
foreach ($l in @(Format-NetworkState -Facts $facts -Result $state)) { Say $l }
if ($expectOffline -and $state.State -eq 'Online') {
  Say "STOP: the label is offline but the machine is online. Turn Wi-Fi off and unplug Ethernet first."
  Write-WbEvidence -Path (Join-Path $OutDir "offline-session-$Label.txt") -Lines $log -MachineLabel $Machine
  exit 3
}

if (-not (Stop-WbApp)) { Say 'STOP: a previous WebView2 is still running.'; exit 3 }
$run = Start-WbApp -Exe $exePath -TimeoutSeconds 30
$app = $run.Process
Say "window shown after $($run.WindowMs) ms; rendered after $($run.RenderMs) ms"
Write-Host ''
Write-Host ">>> Press Start on the Record screen now, record about a minute, press Stop. Watching for $WatchSeconds s."
Write-Host ''

$loop = @('127.0.0.1', '::1')
$seen = @{}
$sw = [System.Diagnostics.Stopwatch]::StartNew()
$polls = 0
$maxProcs = 0
while ($sw.Elapsed.TotalSeconds -lt $WatchSeconds -and -not $app.HasExited) {
  $ids = @((Get-WbProcessTree $app.Id).ProcessId) + @((Get-WbWebView2).ProcessId)
  $maxProcs = [math]::Max($maxProcs, $ids.Count)
  foreach ($c in @(Get-NetTCPConnection -ErrorAction SilentlyContinue | Where-Object { $ids -contains $_.OwningProcess })) {
    if ($c.State -eq 'Listen') {
      if ($c.LocalAddress -notin $loop) { $seen["TCP listen $($c.LocalAddress):$($c.LocalPort)"] = [int]$sw.Elapsed.TotalSeconds }
    } elseif ($c.RemoteAddress -notin ($loop + @('0.0.0.0', '::'))) {
      $seen["$($c.RemoteAddress):$($c.RemotePort) $($c.State)"] = [int]$sw.Elapsed.TotalSeconds
    }
  }
  Get-NetUDPEndpoint -ErrorAction SilentlyContinue |
    Where-Object { $ids -contains $_.OwningProcess -and $_.LocalAddress -notin $loop } |
    ForEach-Object { $seen["UDP $($_.LocalAddress):$($_.LocalPort)"] = [int]$sw.Elapsed.TotalSeconds }
  $polls++
  Start-Sleep -Milliseconds 500
}
$alive = -not $app.HasExited
Say "watched $([int]$sw.Elapsed.TotalSeconds) s, $polls polls, up to $maxProcs processes (app tree plus WebView2); app still running at the end: $alive"
if ($seen.Count -eq 0) {
  Say 'non-loopback sockets seen: none'
} else {
  Say "non-loopback sockets seen: $($seen.Count)"
  foreach ($k in ($seen.Keys | Sort-Object)) { Say "  $k (last seen at +$($seen[$k]) s)" }
}
[void](Stop-WbApp)

if ($expectOffline) {
  if ($seen.Count -eq 0 -and $gateStrings -eq 0 -and $alive) { Say 'verdict: PASS (offline: no non-loopback socket during a recording session)' }
  else { Say 'verdict: FAIL' }
} else {
  Say "verdict: BASELINE (online: $($seen.Count) non-loopback endpoints from WebView2 are expected; compare with the offline run)"
}
Write-WbEvidence -Path (Join-Path $OutDir "offline-session-$Label.txt") -Lines $log -MachineLabel $Machine
Write-Host "log: $(Join-Path $OutDir "offline-session-$Label.txt")"
