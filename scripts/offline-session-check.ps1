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

  The log goes through scripts/lib/Redact.psm1 (the repository is public), and every private network
  address (10/8, 172.16/12, 192.168/16, 169.254/16, fc00::/7, fe80::/10) in the console and in the log is
  replaced by <private-ip> (scripts/lib/OfflineSession.psm1). Masking changes only the text: a socket is
  classified on its real address first, and a non-loopback socket seen offline is a FAIL.

  An offline PASS needs proof that the app post-processed a recording, not only the absence of sockets:
  capability.json in the app data folder, a transcript.db in the session folder (written during the run), and
  an engine process doing post-processing (`batch`) beyond the start-up `info` probes in the watched tree.
  If any is missing the verdict is INCONCLUSIVE (no post-processing: <which>) and the exit code is 4.
  Exit codes: 0 PASS or online baseline, 1 FAIL, 3 STOP (did not start), 4 INCONCLUSIVE.

.EXAMPLE
  powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\offline-session-check.ps1 -Label offline -Machine Legion -OutDir .\docs\evidence\phase-1
#>
[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)] [ValidateSet('online', 'offline')] [string]$Label,
  [string]$Exe,
  [string]$OutDir,
  [int]$WatchSeconds = 240,
  [string]$Machine = '%COMPUTERNAME%',
  # The app data folder (capability.json, settings.json): the release build's own.
  [string]$AppDataDir = (Join-Path $env:APPDATA 'in.edu.bmu.stelmar.voiceworkbench')
)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
Import-Module (Join-Path $here 'lib\PsEnv.psm1') -Force
Set-WbCleanPSModulePath
Set-WbInvariantCulture
if (-not $Exe) { $Exe = Join-Path $here '..\target\release\stelmar-voice-workbench.exe' }
if (-not $OutDir) { $OutDir = Join-Path $here '..\docs\evidence\phase-1' }
Import-Module (Join-Path $here 'lib\NetworkState.psm1') -Force
Import-Module (Join-Path $here 'lib\Redact.psm1') -Force
Import-Module (Join-Path $here 'lib\AppProcess.psm1') -Force
Import-Module (Join-Path $here 'lib\OfflineSession.psm1') -Force

$log = New-Object System.Collections.Generic.List[string]
# Private addresses are masked in every line that is printed or kept; sockets were classified before this.
function Say([string]$line) {
  $masked = $line | ConvertTo-WbPrivateMasked
  Write-Host ($masked | ConvertTo-WbRedacted -MachineLabel $Machine)
  $log.Add($masked)
}

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
# The release build cannot be given a data folder of its own, so it runs on the real app data folder. The three
# files in it (settings.json, capability.json, verified-models.json) are copied aside now and put back byte for
# byte after the checks below have read them (GateData.psm1).
Import-Module (Join-Path $here 'lib\GateData.psm1') -Force
$guard = Save-WbRealAppData -SaveDir (Join-Path ([IO.Path]::GetTempPath()) ('wb-appdata-guard-' + [guid]::NewGuid().ToString('N')))
$run = Start-WbApp -Exe $exePath -TimeoutSeconds 30
$app = $run.Process
Say "window shown after $($run.WindowMs) ms; rendered after $($run.RenderMs) ms"
Write-Host ''
Write-Host ">>> Press Start on the Record screen now, record about a minute, press Stop. Watching for $WatchSeconds s."
Write-Host ''

$loop = @('127.0.0.1', '::1')
$seen = @{}
$sw = [System.Diagnostics.Stopwatch]::StartNew()
$startedAt = Get-Date
$engineCmds = @{}   # subcommand of every wb-engine process seen in the watched tree -> processes seen
$polls = 0
$maxProcs = 0
while ($sw.Elapsed.TotalSeconds -lt $WatchSeconds -and -not $app.HasExited) {
  $ids = @((Get-WbProcessTree $app.Id).ProcessId) + @((Get-WbWebView2).ProcessId)
  $maxProcs = [math]::Max($maxProcs, $ids.Count)
  foreach ($c in @(Get-NetTCPConnection -ErrorAction SilentlyContinue | Where-Object { $ids -contains $_.OwningProcess })) {
    if ($c.State -eq 'Listen') {
      if ($c.LocalAddress -notin $loop) { $seen["TCP listen $(Format-WbEndpoint $c.LocalAddress $c.LocalPort)"] = [int]$sw.Elapsed.TotalSeconds }
    } elseif ($c.RemoteAddress -notin ($loop + @('0.0.0.0', '::'))) {
      $seen["$(Format-WbEndpoint $c.RemoteAddress $c.RemotePort) $($c.State)"] = [int]$sw.Elapsed.TotalSeconds
    }
  }
  Get-NetUDPEndpoint -ErrorAction SilentlyContinue |
    Where-Object { $ids -contains $_.OwningProcess -and $_.LocalAddress -notin $loop } |
    ForEach-Object { $seen["UDP $(Format-WbEndpoint $_.LocalAddress $_.LocalPort)"] = [int]$sw.Elapsed.TotalSeconds }
  # Engines of the app (children of the app tree): which subcommand each runs. `info` is the start-up probe,
  # `batch` is post-processing.
  foreach ($e in @(Get-CimInstance Win32_Process -Filter "Name LIKE 'wb-engine%'" -ErrorAction SilentlyContinue | Where-Object { $ids -contains [int]$_.ParentProcessId })) {
    $sub = Get-WbEngineSubcommand -CommandLine $e.CommandLine
    if (-not $sub) { $sub = 'unknown' }
    if (-not $engineCmds.ContainsKey($sub)) { $engineCmds[$sub] = New-Object 'System.Collections.Generic.HashSet[int]' }
    [void]$engineCmds[$sub].Add([int]$e.ProcessId)
  }
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

# Proof that the app post-processed a recording while it was watched.
$capFile = Join-Path $AppDataDir 'capability.json'
$capOk = Test-Path -LiteralPath $capFile -PathType Leaf
$sessionRoot = $null
try {
  $settingsFile = Join-Path $AppDataDir 'settings.json'
  if (Test-Path -LiteralPath $settingsFile) { $sessionRoot = (Get-Content -Raw -LiteralPath $settingsFile | ConvertFrom-Json).recordings_root }
} catch { $sessionRoot = $null }
$dbs = @()
if ($sessionRoot -and (Test-Path -LiteralPath $sessionRoot)) {
  $dbs = @(Get-ChildItem -LiteralPath $sessionRoot -Recurse -Filter 'transcript.db' -File -ErrorAction SilentlyContinue | Where-Object { $_.LastWriteTime -ge $startedAt.AddSeconds(-5) })
}
$dbOk = ($dbs.Count -gt 0)
$batchOk = ($engineCmds.ContainsKey('batch'))
Say ("capability.json in the app data folder: " + $(if ($capOk) { "present, written $((Get-Item -LiteralPath $capFile).LastWriteTime.ToString('yyyy-MM-dd HH:mm'))" } else { 'MISSING' }))
Say ("transcript.db in the session folder written during the run: " + $(if ($dbOk) { "$($dbs.Count) (first: $($dbs[0].FullName))" } else { 'NONE' }))
$subs = @($engineCmds.Keys | Sort-Object | ForEach-Object { "$_ x$($engineCmds[$_].Count)" })
Say ("engine processes seen in the watched tree: " + $(if ($subs.Count) { $subs -join ', ' } else { 'none' }) + "  (info = start-up probe, batch = post-processing)")

$exitCode = 0
if ($expectOffline) {
  $v = Get-WbOfflineVerdict -NonLoopbackSeen $seen.Count -GateStrings $gateStrings -AppAlive $alive `
    -CapabilityJson $capOk -TranscriptDb $dbOk -EngineBeyondProbes $batchOk
  Say $v.Verdict
  $exitCode = $v.ExitCode
} else {
  Say "verdict: BASELINE (online: $($seen.Count) non-loopback endpoints from WebView2 are expected; compare with the offline run)"
}
foreach ($l in (Restore-WbRealAppData -Saved $guard)) { Say "real app data, $l" }
Remove-Item -LiteralPath $guard.SaveDir -Recurse -Force -ErrorAction SilentlyContinue
Write-WbEvidence -Path (Join-Path $OutDir "offline-session-$Label.txt") -Lines $log -MachineLabel $Machine
Write-Host "log: $(Join-Path $OutDir "offline-session-$Label.txt")"
exit $exitCode
