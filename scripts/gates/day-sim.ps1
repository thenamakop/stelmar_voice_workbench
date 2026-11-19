<#
.SYNOPSIS
  Phase 4 day simulation, in four modes (exactly one):
    -Smoke     a short run that is NOT a gate: the paths of the post-processing queue run together once with the real
               engines (the description below is of this mode).
    -Day       the full day (owner, 18 Nov 2026): four sessions of 90 minutes at the programme's times (10:00, 12:00,
               14:30, 16:00 on the scheduler's clock) played through the cable in real time, a reference capture of
               the cable for each, then the night pass in a 2 h time box whose finish is extrapolated (an ESTIMATE).
               About 11 h. Audio: make_day_audio_full.py --day. Judged by the Phase 1 recording rule per session
               (Fidelity.psm1) and the queue checks of the plan; evidence by day_report.py.
    -Rehearse  the same schedule compressed (four sessions of 2 minutes, a night box of 8 minutes): proves the script
               end to end in under 30 minutes. NOT a gate. Audio: make_day_audio_full.py --rehearse.
    -Kill      the engine-kill test (queue-kill.ps1 calls it).
  The quiet-machine rule is checked with the gate lock held (Wait-WbQuiet; idle-check.json in the run folder).

.DESCRIPTION
  -Smoke runs the gate build of the app (target\gate, cargo feature gate-hooks; never an installer build) inside the
  emulation job (wb-emulate, default emu-low) with WB_GATE_DAY_PLAN. The app records three short sessions from the
  virtual cable (scripts\gates\make_day_audio.py --smoke builds the audio: English, Hindi, and a session that changes
  from English to Hindi and back), one wb-testbench player per session, started outside the job when the app logs the
  session's start. The plan also does, at fixed moments:

    - queue_pause and queue_resume after the first session stopped (the operator's pause and resume),
    - set_language_lock to Hindi and back to detection inside the third session (the lock must reach the lang_log
      of the unit that covers it: read from the batch request files afterwards),
    - import_session of a copy of the first finished session (a folder as it would arrive from another computer).

  The script itself kills one batch engine process from outside (taskkill, as the supervisor kill test does) once an
  engine of the second or third session has run for 20 s. The night pass is made to start 60 s after the last
  session (WB_GATE_NIGHT_WAIT_S, gate builds only) and is stopped after -NightMinutes minutes: "a short night pass".

  The saved profile of a full system check (D:\STELMAR-Gate\system-check\full-emu-low\data) is copied into a data
  folder of this run, so the real profile is never touched. The app's own view of it must show no fingerprint
  change, or the run stops (a stale profile would decide with safe defaults and test nothing else).

  The run holds D:\Projects\stelmar-gate.lock (Invoke-WbGate), refuses to start while overlay tools or builds run,
  and the evidence goes through scripts\lib\Redact.psm1. Audio stays under D:\STELMAR-Gate and C:\STELMAR-Gate-Backup,
  never in the repository.

  Needs: VB-CABLE (run scripts\gates\rig-selftest.ps1 first after a restart), the gate build
  (scripts\gates\build-gate.ps1), the engines in src-tauri\binaries, wb-testbench and wb-emulate release builds, the
  models under models\, and the smoke audio (eval\.venv\Scripts\python scripts\gates\make_day_audio.py --smoke).

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File .\scripts\gates\day-sim.ps1 -Smoke
  powershell -ExecutionPolicy Bypass -File .\scripts\gates\day-sim.ps1 -Smoke -FromRun D:\STELMAR-Gate\day-sim\smoke-emu-low-20261114-231500
#>
[CmdletBinding()]
param(
  [switch]$Smoke,
  # The full day (four sessions at the programme's times, the night box), its compressed rehearsal and the engine-kill
  # test (owner, 18 Nov 2026, plan step 7). Exactly one of -Smoke, -Day, -Rehearse and -Kill.
  [switch]$Day,
  [switch]$Rehearse,
  [switch]$Kill,
  [ValidateSet('emu-low', 'emu-mid')] [string]$Profile = 'emu-low',
  [string]$Machine = 'Legion',
  [string]$WorkRoot = 'D:\STELMAR-Gate\day-sim',
  [string]$BackupRoot = 'C:\STELMAR-Gate-Backup',
  [string]$ProfileData = 'D:\STELMAR-Gate\system-check-19nov\full-emu-low\data',
  [string]$Audio = 'D:\STELMAR-Gate\day-sim\smoke',
  [int]$NightMinutes = 8,
  # Longest the whole run may take (the app's start to the stop); a longer session or night box needs more.
  [int]$MaxMinutes = 0,
  # Minutes the night decision may take after the last session (the rolling backlog comes first); 0 = per mode.
  [int]$DecisionMinutes = 0,
  # Names the run folder and the evidence file: smoke (the first smoke) or a second smoke with other audio.
  [string]$Tag = 'smoke',
  [int]$PartSeconds = 120,
  [string]$CaptureDevice = '',
  [string]$PlayDevice = '',
  [string]$OutDir,
  [string]$Exe,
  [string]$Testbench,
  [string]$Emulate,
  [switch]$AllowCableDefault,
  # Write the evidence again from a finished run's folder; start nothing.
  [string]$FromRun
)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo = (Resolve-Path (Join-Path $here '..\..')).Path
$modeSwitches = @($Smoke, $Day, $Rehearse, $Kill | Where-Object { $_ })
if ($modeSwitches.Count -ne 1) { throw 'choose exactly one of -Smoke, -Day, -Rehearse and -Kill' }
$mode = $(if ($Day) { 'day' } elseif ($Rehearse) { 'rehearse' } elseif ($Kill) { 'kill' } else { 'smoke' })
$fidelity = ($mode -eq 'day' -or $mode -eq 'rehearse')
if (-not $PSBoundParameters.ContainsKey('Tag')) { $Tag = $mode }
if (-not $PSBoundParameters.ContainsKey('Audio')) {
  $Audio = @{ smoke = 'D:\STELMAR-Gate\day-sim\smoke'; day = 'D:\STELMAR-Gate\day-sim\day-audio'; rehearse = 'D:\STELMAR-Gate\day-sim\day-audio-rehearse'; kill = 'D:\STELMAR-Gate\day-sim\kill-audio' }[$mode]
}
if (-not $PSBoundParameters.ContainsKey('NightMinutes')) { $NightMinutes = @{ smoke = 8; day = 120; rehearse = 8; kill = 0 }[$mode] }
if (-not $PSBoundParameters.ContainsKey('PartSeconds') -and $mode -eq 'rehearse') { $PartSeconds = 60 }
if ($MaxMinutes -le 0) { $MaxMinutes = @{ smoke = 100; day = 900; rehearse = 45; kill = 240 }[$mode] }
if ($DecisionMinutes -le 0) { $DecisionMinutes = @{ smoke = 40; day = 360; rehearse = 20; kill = 240 }[$mode] }
if (-not $OutDir) { $OutDir = Join-Path $repo 'docs\evidence\phase-4' }
if (-not $Exe) { $Exe = Join-Path $repo 'target\gate\stelmar-voice-workbench.exe' }
if (-not $Testbench) { $Testbench = Join-Path $repo 'target\release\wb-testbench.exe' }
if (-not $Emulate) { $Emulate = Join-Path $repo 'target\release\wb-emulate.exe' }
Import-Module (Join-Path $repo 'scripts\lib\PsEnv.psm1') -Force
Set-WbCleanPSModulePath
Set-WbInvariantCulture
Import-Module (Join-Path $repo 'scripts\lib\GateRun.psm1') -Force
Import-Module (Join-Path $repo 'scripts\lib\Redact.psm1') -Force
Import-Module (Join-Path $repo 'scripts\lib\AppProcess.psm1') -Force
Import-Module (Join-Path $repo 'scripts\lib\Rig.psm1') -Force
Import-Module (Join-Path $repo 'scripts\lib\CableRig.psm1') -Force
Import-Module (Join-Path $repo 'scripts\lib\Fidelity.psm1') -Force
Import-Module (Join-Path $repo 'scripts\lib\Quiet.psm1') -Force
$python = Join-Path $repo 'eval\.venv\Scripts\python.exe'
New-Item -ItemType Directory -Force -Path $OutDir, $WorkRoot | Out-Null

function Q([string]$v) { ConvertTo-WbArg $v }

function Read-Json([string]$Path) {
  if (Test-Path -LiteralPath $Path) { Get-Content -LiteralPath $Path -Raw -Encoding UTF8 | ConvertFrom-Json } else { $null }
}

function Write-Utf8([string]$Path, [string]$Text) {
  [IO.File]::WriteAllText($Path, $Text, (New-Object Text.UTF8Encoding($false)))
}

# Per-process samples (owner, 18 Nov 2026): every 10 s one row of CPU (percent of the whole host) and GPU (percent
# of an engine, 3D and compute engines of each process summed) by who used it: the batch engines, the app, its
# WebView2 processes, the test player, this script (the sampler's own cost) and everything else (host total less
# the named ones). It tells how much of a unit's slowdown against a quiet run is our own app and how much is noise
# outside it. day_sim_report.py reads process-samples.csv for each unit's window.
$script:SampleCpu = @{}
$script:SampleAt = $null
$script:SampleTree = @()
$script:SampleTreeAt = $null
$script:HostCounter = $null

function Add-ProcessSample {
  param([string]$Path, [int]$RootId)
  if (-not $script:HostCounter) {
    $script:HostCounter = New-Object System.Diagnostics.PerformanceCounter('Processor', '% Processor Time', '_Total')
    $null = $script:HostCounter.NextValue()
  }
  $now = Get-Date
  if (-not $script:SampleTreeAt -or ($now - $script:SampleTreeAt).TotalSeconds -ge 60) {
    $script:SampleTree = @(Get-WbProcessTree $RootId | ForEach-Object { [int]$_.ProcessId })
    $script:SampleTreeAt = $now
  }
  $procs = @(Get-Process -ErrorAction SilentlyContinue)
  $cat = @{}
  $cpu = @{ engine = 0.0; app = 0.0; webview = 0.0; player = 0.0; script = 0.0 }
  $dt = if ($script:SampleAt) { ($now - $script:SampleAt).TotalSeconds } else { 0 }
  $cores = [Environment]::ProcessorCount
  $next = @{}
  foreach ($p in $procs) {
    $name = $p.ProcessName
    $c = $null
    if ($p.Id -eq $PID) { $c = 'script' }
    elseif ($name -like 'wb-engine*') { $c = 'engine' }
    elseif ($name -eq 'stelmar-voice-workbench') { $c = 'app' }
    elseif ($name -eq 'msedgewebview2' -and ($script:SampleTree -contains $p.Id)) { $c = 'webview' }
    elseif ($name -eq 'wb-testbench') { $c = 'player' }
    if (-not $c) { continue }
    $cat[$p.Id] = $c
    $t = try { $p.TotalProcessorTime.TotalSeconds } catch { $null }
    if ($null -eq $t) { continue }
    $next[$p.Id] = $t
    if ($dt -gt 0 -and $script:SampleCpu.ContainsKey($p.Id)) {
      $cpu[$c] += [math]::Max(0.0, $t - $script:SampleCpu[$p.Id]) / $dt / $cores * 100.0
    }
  }
  $script:SampleCpu = $next
  $script:SampleAt = $now
  $hostCpu = [double]$script:HostCounter.NextValue()
  $named = $cpu.engine + $cpu.app + $cpu.webview + $cpu.player + $cpu.script
  $other = [math]::Max(0.0, $hostCpu - $named)
  # GPU: sum of the 3D and compute engines of each process, by category; the rest of the card is 'other'.
  $gpu = @{ engine = 0.0; app = 0.0; webview = 0.0; other = 0.0 }
  $byLuid = @{}
  try {
    $samples = (Get-Counter '\GPU Engine(*)\Utilization Percentage' -ErrorAction Stop).CounterSamples
    foreach ($g in $samples) {
      if ($g.InstanceName -notmatch '^pid_(\d+)_.*engtype_(3D|Compute)') { continue }
      $gid = [int]$Matches[1]
      $k = if ($cat.ContainsKey($gid) -and $gpu.ContainsKey($cat[$gid])) { $cat[$gid] } else { 'other' }
      $gpu[$k] += [double]$g.CookedValue
      if ($g.InstanceName -match 'luid_0x([0-9a-fA-F]+)_0x([0-9a-fA-F]+)') {
        $lk = "$($Matches[1]).$($Matches[2])"
        if (-not $byLuid.ContainsKey($lk)) { $byLuid[$lk] = @{ engine = 0.0; app = 0.0; webview = 0.0; other = 0.0 } }
        $byLuid[$lk][$k] += [double]$g.CookedValue
      }
    }
  } catch { }
  if ($dt -gt 0) {
    if (-not (Test-Path -LiteralPath $Path)) {
      Set-Content -LiteralPath $Path -Encoding ascii -Value 'unix_s,host_cpu,engine_cpu,app_cpu,webview_cpu,player_cpu,script_cpu,other_cpu,engine_gpu,app_gpu,webview_gpu,other_gpu,gpu_by_adapter'
    }
    # Per adapter (luid): engine/app/webview/other in percent, adapters separated by ';'.
    $ad = ($byLuid.GetEnumerator() | Sort-Object Name | ForEach-Object { '{0}={1:F0}/{2:F0}/{3:F0}/{4:F0}' -f $_.Name, $_.Value.engine, $_.Value.app, $_.Value.webview, $_.Value.other }) -join ';'
    $row = '{0},{1:F1},{2:F1},{3:F1},{4:F1},{5:F1},{6:F1},{7:F1},{8:F1},{9:F1},{10:F1},{11:F1},{12}' -f [DateTimeOffset]::UtcNow.ToUnixTimeSeconds(), $hostCpu, $cpu.engine, $cpu.app, $cpu.webview, $cpu.player, $cpu.script, $other, $gpu.engine, $gpu.app, $gpu.webview, $gpu.other, $ad
    Add-Content -LiteralPath $Path -Encoding ascii -Value $row
  }
}

# The new lines of a text file another process appends to (shared read).
function Read-NewLines([string]$Path, [ref]$Count) {
  if (-not (Test-Path -LiteralPath $Path)) { return @() }
  $fs = [IO.File]::Open($Path, 'Open', 'Read', 'ReadWrite')
  try {
    $sr = New-Object IO.StreamReader($fs, [Text.Encoding]::UTF8)
    $all = @(($sr.ReadToEnd()) -split "`r?`n" | Where-Object { $_ -ne '' })
  } finally { $fs.Dispose() }
  $new = @($all | Select-Object -Skip $Count.Value)
  $Count.Value = $all.Count
  $new
}

# Each recorded session judged by the Phase 1 recording rule (scripts\lib\Fidelity.psm1, docs/PLAN.md section 12):
# the app's recording against the reference capture of the same cable, one fixed offset per run, 0 dropped samples,
# upstream breaks counted against max(1, floor(minutes / 10)); an INVALID run is reported as such, not repeated here.
function Invoke-FidelityJudging {
  param([string]$Run, $Refs, $ReportJson)
  $results = @()
  foreach ($r in @($Refs)) {
    $fid = Join-Path $Run "fid-$($r.Index)"
    New-Item -ItemType Directory -Force -Path $fid | Out-Null
    $sess = @(@($ReportJson.sessions) | Where-Object { $_.title -eq $r.Title })[0]
    if (-not $sess) {
      $results += [ordered]@{ title = $r.Title; verdict = 'NOT RECORDED'; reasons = @('the app did not report this session') }
      continue
    }
    $sessionDir = [string]$sess.dir
    $null = Invoke-WbAnalyse -Testbench $Testbench -Session $sessionDir -Source $r.Source -Report (Join-Path $fid 'analysis.json')
    $null = Invoke-WbAnalyse -Testbench $Testbench -Session $r.Dir -Source $r.Source -Report (Join-Path $fid 'reference-analysis.json')
    $cmp = Invoke-WbCompare -Testbench $Testbench -SessionDir $sessionDir -ReferenceDir $r.Dir -RunDir $fid
    $compare = Read-Json (Join-Path $fid 'compare.json')
    $refAnalysis = Read-Json (Join-Path $fid 'reference-analysis.json')
    $play = Read-Json (Join-Path $Run "play-$($r.Index).json")
    $reference = Read-Json (Join-Path $Run "reference-$($r.Index).json")
    $dropped = [int64]0
    $overrun = [int64]0
    $xruns = [int64]0
    if ($sess.stopped -and $sess.stopped.recorder) {
      $dropped = [int64]$sess.stopped.recorder.dropped_samples
      $overrun = [int64]$sess.stopped.recorder.overrun_samples
      $xruns = [int64]$sess.stopped.recorder.xruns
    }
    $verdict = $null
    $lines = @()
    try {
      $playStart = [DateTimeOffset]::Parse($play.started_at)
      $verdict = Get-WbRunVerdict -Compare $compare -RefAnalysis $refAnalysis -Play $play -Reference $reference -PlayStarted $playStart -DroppedSamples $dropped
      $lines = @(Get-WbVerdictLines -Result $verdict -Compare $compare)
    } catch {
      $lines = @("the verdict could not be worked out: $($_.Exception.Message)")
    }
    Write-Utf8 (Join-Path $Run "fidelity-$($r.Index).txt") ($lines -join "`r`n")
    $results += [ordered]@{
      title = $r.Title; index = $r.Index
      verdict = $(if ($verdict) { $verdict.Verdict } else { 'NO VERDICT' })
      audio_minutes = $(if ($verdict) { [math]::Round($verdict.AudioMinutes, 1) } else { $null })
      upstream_breaks = $(if ($verdict) { @($verdict.Breaks).Count } else { $null })
      counted_breaks = $(if ($verdict) { @($verdict.CountedBreaks).Count } else { $null })
      break_limit = $(if ($verdict) { $verdict.BreakLimit } else { $null })
      invalid_reasons = $(if ($verdict) { @($verdict.InvalidReasons) } else { @() })
      fail_reasons = $(if ($verdict) { @($verdict.FailReasons) } else { @() })
      compare_aligned = $(if ($compare) { [bool]$compare.aligned } else { $null })
      runs_of_the_app = $(if ($compare) { @($compare.segments).Count } else { $null })
      differing_frames = $(if ($compare) { [int64]$compare.differing_frames } else { $null })
      dropped_samples = $dropped; overrun_samples = $overrun; xruns = $xruns
    }
  }
  Write-Utf8 (Join-Path $Run 'fidelity.json') ((@{ sessions = @($results) } | ConvertTo-Json -Depth 6))
  $results
}

function Write-SmokeEvidence([string]$Run) {
  $tmp = Join-Path $Run 'evidence.txt'
  $svg = Join-Path $Run 'timeline.svg'
  Remove-Item -LiteralPath $tmp, $svg -Force -ErrorAction SilentlyContinue
  $reportPy = $(if ($mode -eq 'smoke') { 'day_sim_report.py' } else { 'day_report.py' })
  $reportArgs = @('-X', 'utf8', (Join-Path $repo "scripts\gates\$reportPy"), $Run, $tmp)
  if ($mode -ne 'smoke') { $reportArgs += @('--mode', $mode) }
  $rep = Invoke-WbNative -FilePath $python -ArgumentList $reportArgs
  if (-not (Test-Path -LiteralPath $tmp)) { throw "the report was not written: $((@($rep.Error) + @($rep.Output)) -join ' ')" }
  $body = @([IO.File]::ReadAllLines($tmp, [Text.Encoding]::UTF8))
  $base = $(if ($mode -eq 'smoke') { "day-sim-$Tag-$Profile" } elseif ($mode -eq 'day') { "day-sim-$Profile" } elseif ($mode -eq 'rehearse') { "day-sim-rehearsal-$Profile" } else { "queue-kill-$Profile" })
  $out = Join-Path $OutDir "$base.txt"
  $switch = @{ smoke = '-Smoke'; day = '-Day'; rehearse = '-Rehearse'; kill = '-Kill' }[$mode]
  $purpose = $(if ($mode -eq 'smoke') { 'NOT A GATE (a smoke run of the queue paths)' } elseif ($mode -eq 'rehearse') { 'NOT A GATE (the whole schedule compressed, to prove the script end to end)' } else { 'PROVISIONAL (emulation profile; the real laptops repeat it)' })
  $header = @(
    "date: $(Get-Date -Format 'yyyy-MM-dd HH:mm') (IST on the Legion)",
    "command: powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\gates\day-sim.ps1 $switch -Profile $Profile -Tag $Tag -Audio $Audio -NightMinutes $NightMinutes -MaxMinutes $MaxMinutes -DecisionMinutes $DecisionMinutes",
    "machine: $Machine, emulation profile $Profile; the gate build target\gate\stelmar-voice-workbench.exe, WB_GATE_DAY_PLAN; $purpose",
    "source: $((Invoke-WbNative -FilePath 'git' -ArgumentList @('-C', $repo, 'rev-parse', '--short', 'HEAD')).Output -join '')",
    "build: $(Split-Path -Leaf $Exe) sha256 $((Get-FileHash -LiteralPath $Exe -Algorithm SHA256).Hash.ToLowerInvariant())",
    "run folder: $Run"
  )
  Write-WbEvidence -Path $out -Lines ($header + $body)
  if (Test-Path -LiteralPath $svg) { Copy-Item -LiteralPath $svg -Destination (Join-Path $OutDir "$base-timeline.svg") -Force }
  Write-Host "evidence: $out"
}

if ($FromRun) {
  Write-SmokeEvidence (Resolve-Path $FromRun).Path
  return
}

foreach ($f in @($Exe, $Testbench, $Emulate)) { if (-not (Test-Path -LiteralPath $f)) { throw "Missing $f; build it first (scripts\gates\build-gate.ps1)" } }
$audioManifest = Join-Path $Audio $(if ($mode -eq 'smoke') { 'smoke-audio.json' } else { 'day-audio.json' })
if (-not (Test-Path -LiteralPath $audioManifest)) {
  $how = @{ smoke = 'make_day_audio.py --smoke'; day = 'make_day_audio_full.py --day'; rehearse = 'make_day_audio_full.py --rehearse'; kill = 'make_day_audio_full.py --kill-test' }[$mode]
  throw "no audio at $Audio; run: eval\.venv\Scripts\python scripts\gates\$how"
}
foreach ($f in @('capability.json')) { if (-not (Test-Path -LiteralPath (Join-Path $ProfileData $f))) { throw "no saved profile at $ProfileData (run scripts\gates\system-check-gate.ps1 -Runs full-emu-low)" } }
$sessions = @((Get-Content -LiteralPath $audioManifest -Raw -Encoding UTF8 | ConvertFrom-Json).sessions)

$failed = $null
try {
  $result = Invoke-WbGate -Name "day-sim $mode $Profile" -Body {
    if (Get-Process stelmar-voice-workbench -ErrorAction SilentlyContinue) { throw 'The app is already running; close it first.' }
    # The rig first (see recorder-gate.ps1): paired endpoints, the cable not the default device, nobody else playing
    # into it, a known signal heard on the capture.
    Assert-WbNoResidentAudio
    $cableRig = Resolve-WbCableRig -Testbench $Testbench -PlayDevice $PlayDevice -CaptureDevice $CaptureDevice
    $null = @(Assert-WbCableNotDefault -Testbench $Testbench -Rig $cableRig -Allow:$AllowCableDefault)
    $intruders = @(Get-WbCableIntruders -Testbench $Testbench -Names $cableRig.RenderNames)
    if ($intruders.Count -gt 0) { throw ("RIG REFUSED: another program is playing into the cable: " + ($intruders -join ', ')) }
    $workRootForTest = Split-Path -Parent $WorkRoot
    $cableTest = Invoke-WbRigSelfTest -Rig $cableRig -Testbench $Testbench -WorkRoot $workRootForTest
    $playName = $cableTest.PlayName
    $capName = $cableTest.CaptureName
    $rigPlan = Get-WbRigPlan -Emulate $Emulate -Profile $Profile
    if ($rigPlan) { Assert-WbRigPlacement -JobMask $rigPlan.JobMask -PlayerMask $rigPlan.PlayerMask -ReferenceMask $rigPlan.ReferenceMask -What 'planned, before anything starts' }

    $stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
    $run = Join-Path $WorkRoot "$Tag-$Profile-$stamp"
    $rec = Join-Path $run 'rec'
    $data = Join-Path $run 'data'
    $backup = Join-Path $BackupRoot "$Tag-$Profile-$stamp"
    New-Item -ItemType Directory -Force -Path $rec, $data | Out-Null
    Copy-Item -LiteralPath (Join-Path $ProfileData 'capability.json') -Destination $data
    if (Test-Path -LiteralPath (Join-Path $ProfileData 'verified-models.json')) { Copy-Item -LiteralPath (Join-Path $ProfileData 'verified-models.json') -Destination $data }
    Write-Host "$(Get-Date -Format 'HH:mm:ss') run folder $run"
    # Room for the recordings, their mirror and the work files before four sessions of 90 minutes start.
    # The recordings (and the work files, the night pass's copies and imports) are on the run drive, their mirror on the backup drive.
    $needRec = @{ smoke = 4; day = 14; rehearse = 2; kill = 8 }[$mode]
    $needMirror = @{ smoke = 2; day = 6; rehearse = 1; kill = 3 }[$mode]
    foreach ($pair in @(@($rec, $needRec), @($backup, $needMirror))) {
      $drive = (Get-Item -LiteralPath (Split-Path -Qualifier $pair[0]) -ErrorAction Stop).PSDrive
      $freeGb = [math]::Round($drive.Free / 1GB, 1)
      Write-Host "$(Get-Date -Format 'HH:mm:ss') free space on $($drive.Name): $freeGb GB (this run needs about $($pair[1]) GB there)"
      if ($freeGb -lt $pair[1]) { throw "only $freeGb GB free on $($drive.Name): ; the $mode run needs about $($pair[1]) GB there" }
    }
    # The quiet-machine rule, with the gate lock held: nothing starts on a busy machine.
    $quiet = Wait-WbQuiet -Out (Join-Path $run 'idle-check.json')
    Write-Host "$(Get-Date -Format 'HH:mm:ss') quiet-machine rule: $quiet"
    # The driver guard (owner, 19 Nov 2026): the Intel and NVIDIA display drivers are recorded at the start and the end
    # and checked every minute; a change stops the run and marks it INVALID (a profile is only valid for one driver).
    $driversStart = Get-WbGpuDrivers
    $driverChange = $null
    $lastDriverCheck = Get-Date
    Write-Host "$(Get-Date -Format 'HH:mm:ss') display drivers at the start: $driversStart"

    function Start-Emulated([string]$Mode, [int]$LimitMinutes) {
      $emu = Start-Process -FilePath $Emulate -ArgumentList @('--profile', $Profile, '--log', (Q (Join-Path $run "emulate-$Mode.log")), '--', (Q $Exe)) -PassThru -WindowStyle Hidden
      $emu
    }

    # 1. The app's own view of the saved profile: a fingerprint change would decide with safe defaults.
    $env:WB_GATE_DATA_DIR = $data
    $env:WB_GATE_ROOT = $rec
    $env:WB_GATE_CHECK = 'view'
    $env:WB_GATE_REPORT = Join-Path $run 'view.json'
    $view = Start-Emulated 'view' 5
    if (-not $view.WaitForExit(300000)) { Stop-Process -Id $view.Id -Force; throw 'the profile view did not finish within 5 minutes' }
    Remove-Item Env:\WB_GATE_CHECK -ErrorAction SilentlyContinue
    $v = Read-Json (Join-Path $run 'view.json')
    if (-not $v -or -not $v.view.capability) { throw "the app could not read the saved profile (see $run\view.json)" }
    $changes = @($v.view.fingerprint_changes)
    $d = $v.view.capability.decision
    Write-Host ("{0} profile: tier {1}, config {2}, cpu threads {3}, process while recording {4}, fingerprint changes: {5}" -f (Get-Date -Format 'HH:mm:ss'), $d.tier, ($d.config.config | ConvertTo-Json -Compress), $d.config.cpu_threads, $d.process_while_recording, $changes.Count)
    if ($changes.Count -gt 0) { throw "the saved profile no longer matches this build (fingerprint changes: $($changes -join '; ')); run the full check again (scripts\gates\system-check-gate.ps1 -Runs full-emu-low)" }

    # 2. The plan.
    $at = 0
    $planSessions = @()
    $prevEnd = -1
    foreach ($s in $sessions) {
      $dur = [int][math]::Ceiling([double]$s.seconds) + 25
      if ($mode -eq 'day' -or $mode -eq 'rehearse') {
        # The programme's times (10:00, 12:00, 14:30 and 16:00 on the scheduler's clock; compressed in the rehearsal).
        if ([int]$s.at_s -lt $prevEnd) { throw "the sessions overlap: '$($s.title)' starts at $($s.at_s) s, the one before it ends at $prevEnd s" }
        $planSessions += [ordered]@{ title = $s.title; hall = $s.hall; at_s = [int]$s.at_s; duration_s = $dur; languages = @($s.languages); script = $s.script; device = $capName }
        $prevEnd = [int]$s.at_s + $dur
      } else {
        $planSessions += [ordered]@{ title = $s.title; at_s = $at; duration_s = $dur; languages = @($s.languages); device = $capName }
        $at += $dur + 30
      }
    }
    $first = $sessions[0].title
    # The first smoke (three sessions) also pauses, locks a language and imports; a single long session does none
    # of that: it exists to show a unit decoded before Stop and a night pass that commits post-2. The day and its
    # rehearsal do nothing to the queue: they judge what it does by itself. The kill test imports a copy of each
    # session once the kills are done, so the same audio is decoded again without interruption for comparison.
    $actions = @()
    if ($mode -eq 'smoke' -and $sessions.Count -ge 3) {
      $mixed = $sessions[2].title
      $actions = @(
        [ordered]@{ session = $first; on = 'stop'; after_s = 6; do = 'queue_pause' },
        [ordered]@{ session = $first; on = 'stop'; after_s = 50; do = 'queue_resume' },
        [ordered]@{ session = $mixed; on = 'start'; after_s = 170; do = 'lock_language'; lang = 'hi' },
        [ordered]@{ session = $mixed; on = 'start'; after_s = 230; do = 'lock_language' },
        [ordered]@{ session = $first; on = 'stop'; after_s = 120; do = 'import_session'; from = $first }
      )
    }
    if ($mode -eq 'kill') {
      $lastTitle = $sessions[$sessions.Count - 1].title
      $n = 0
      foreach ($s in $sessions) {
        $actions += [ordered]@{ session = $lastTitle; on = 'stop'; after_s = (900 + 15 * $n); do = 'import_session'; from = $s.title }
        $n++
      }
    }
    $planFile = Join-Path $run 'day-plan.json'
    Write-Utf8 $planFile ((@{ sessions = $planSessions; actions = $actions } | ConvertTo-Json -Depth 6))

    # 3. The run: the first session of the plan is 10:00 IST on the scheduler's clock.
    $nowIst = Get-Date
    $ten = (Get-Date -Hour 10 -Minute 0 -Second 0 -Millisecond 0)
    $offset = [int](($ten - $nowIst).TotalSeconds)
    $stopFile = Join-Path $run 'STOP'
    $queueLog = Join-Path $run 'queue.log'
    $env:WB_GATE_DAY_PLAN = $planFile
    $env:WB_GATE_DAY_STOP_FILE = $stopFile
    $env:WB_GATE_REPORT = Join-Path $run 'report.json'
    $env:WB_GATE_BACKUP = $backup
    $env:WB_GATE_QUEUE_LOG = $queueLog
    $env:WB_GATE_CLOCK_OFFSET_S = "$offset"
    # The day's night pass waits the real 30 minutes after the last session; the smoke and the rehearsal 60 seconds.
    if ($mode -eq 'smoke' -or $mode -eq 'rehearse') { $env:WB_GATE_NIGHT_WAIT_S = '60' }
    # The kill test needs both engines at once, and neither emulation profile chose a hybrid: forced and labelled.
    if ($mode -eq 'kill') { $env:WB_GATE_FORCE_HYBRID = '1' }
    $env:WB_GATE_PART_SECONDS = "$PartSeconds"
    $emu = $null
    $players = @()
    $refs = @()
    $rigConfirmed = $false
    $kill = $null
    $kills = @()
    $killKinds = @('avx2', 'vulkan', 'avx2', 'vulkan', 'avx2', 'vulkan', 'avx2', 'vulkan', 'avx2', 'vulkan')
    $killRng = New-Object System.Random 4242
    $nextKillAt = $null
    $killWaitSince = $null
    $lastKillProbe = Get-Date
    $rollingAllAt = $null
    $started = Get-Date
    $hardLimit = $started.AddMinutes($MaxMinutes)
    $lines = 0
    $startedTitles = @{}
    $allRecorded = $null
    $nightPlanned = $null
    $nightSkipped = $null
    $nightDone = $null
    $nightExpected = 0
    $nightCompleted = 0
    $rollingCompleted = 0
    $lastProgress = Get-Date
    $lastSample = Get-Date
    $sampleFile = Join-Path $run 'process-samples.csv'
    $lastLine = ''
    $progress = New-Object System.Collections.Generic.List[string]
    try {
      $emu = Start-Emulated 'day' 100
      while (-not $emu.HasExited) {
        $ref = [ref]$lines
        foreach ($l in (Read-NewLines $queueLog $ref)) {
          $lastLine = $l
          if ($l -match 'DAY: started "(.+?)" in') {
            $title = $Matches[1]
            if (-not $startedTitles.ContainsKey($title)) {
              $startedTitles[$title] = Get-Date
              $s = @($sessions | Where-Object { $_.title -eq $title })[0]
              $i = $startedTitles.Count
              $goFile = Join-Path $run "GO-$i"
              $source = Join-Path $Audio $s.file
              if ($fidelity) {
                # One independent reference capture of the cable per session, outside the job (wb-testbench capture).
                $refDir = Join-Path $run "reference-$i"
                $refStop = Join-Path $run "STOP-REF-$i"
                $refProc = Start-Process -FilePath $Testbench -ArgumentList @('capture', '--device', (Q $capName), '--dir', (Q $refDir), '--stop-file', (Q $refStop), '--go-file', (Q $goFile), '--report', (Q (Join-Path $run "reference-$i.json"))) -PassThru -WindowStyle Hidden -RedirectStandardOutput (Join-Path $run "reference-$i.out") -RedirectStandardError (Join-Path $run "reference-$i.err")
                Start-Sleep -Seconds 1
              }
              $play = Start-Process -FilePath $Testbench -ArgumentList @('play', '--device', (Q $playName), '--file', (Q $source), '--go-file', (Q $goFile), '--report', (Q (Join-Path $run "play-$i.json"))) -PassThru -WindowStyle Hidden -RedirectStandardOutput (Join-Path $run "play-$i.out") -RedirectStandardError (Join-Path $run "play-$i.err")
              if ($fidelity) {
                $rig = Set-WbRigPins -Plan $rigPlan -Player $play -Reference $refProc -GoFile $goFile
                if (-not $rigConfirmed) { $rig = Confirm-WbRigAgainstJob -Rig $rig -Player $play -Reference $refProc; $rigConfirmed = $true }
                $refs += [pscustomobject]@{ Index = $i; Title = $title; Proc = $refProc; Stop = $refStop; Dir = $refDir; Source = $source }
              } else {
                if ($rigPlan) { $null = Set-WbRigPlacement -Process $play -Mask $rigPlan.PlayerMask -Role 'player' }
                Set-Content -LiteralPath $goFile -Value 'go' -Encoding ascii
              }
              $players += $play
              $progress.Add("$(Get-Date -Format 'HH:mm:ss') player $i started for $title ($($s.seconds) s)")
            }
          }
          if ($l -match 'DAY: stopped "(.+?)"') {
            $stoppedTitle = $Matches[1]
            foreach ($r in @($refs | Where-Object { $_.Title -eq $stoppedTitle })) { Set-Content -LiteralPath $r.Stop -Value 'stop' -Encoding ascii }
            $progress.Add("$(Get-Date -Format 'HH:mm:ss') $l")
          }
          if ($l -match 'every session of the plan was recorded') { $allRecorded = Get-Date; $progress.Add("$(Get-Date -Format 'HH:mm:ss') all sessions recorded") }
          if ($l -match 'night pass planned for (\d+) session') { $nightPlanned = Get-Date; $nightExpected = [int]$Matches[1]; $progress.Add("$(Get-Date -Format 'HH:mm:ss') $l") }
          if ($l -match 'post-1 transcript complete') { $rollingCompleted++; $progress.Add("$(Get-Date -Format 'HH:mm:ss') $l") }
          if ($l -match 'post-2 transcript complete') {
            $nightCompleted++
            if ($nightExpected -gt 0 -and $nightCompleted -ge $nightExpected) { $nightDone = Get-Date }
            $progress.Add("$(Get-Date -Format 'HH:mm:ss') $l")
          }
          if ($l -match 'night pass skipped') { $nightSkipped = Get-Date; $progress.Add("$(Get-Date -Format 'HH:mm:ss') $l") }
        }
        $lines = $ref.Value

        # One batch engine killed from outside, once an engine of the second or third session has run 20 s.
        if ($mode -eq 'smoke' -and -not $kill -and $startedTitles.Count -ge 2) {
          $tree = @(Get-WbProcessTree $emu.Id | ForEach-Object { [int]$_.ProcessId })
          $eng = @(Get-CimInstance Win32_Process -Filter "Name like 'wb-engine%'" -ErrorAction SilentlyContinue | Where-Object { $tree -contains [int]$_.ProcessId -and $_.CommandLine -match '\sbatch(\s|$)' })
          foreach ($e in $eng) {
            $age = ((Get-Date) - [datetime]$e.CreationDate).TotalSeconds
            if ($age -ge 20) {
              $kr = Invoke-WbNative -FilePath 'taskkill.exe' -ArgumentList @('/F', '/PID', "$($e.ProcessId)")
              $kill = [pscustomobject]@{ At = (Get-Date -Format 'HH:mm:ss'); Pid = $e.ProcessId; Name = $e.Name; AgeS = [math]::Round($age, 1); Exit = $kr.ExitCode }
              Add-Content -LiteralPath (Join-Path $run 'script-events.log') -Value ("{0}`tSMOKE: killed engine pid {1} ({2}) after {3:n0} s, taskkill exit {4}" -f (Get-Date -Format 'HH:mm'), $e.ProcessId, $e.Name, $age, $kr.ExitCode) -Encoding UTF8
              $progress.Add("$(Get-Date -Format 'HH:mm:ss') killed engine pid $($e.ProcessId) $($e.Name) after $([math]::Round($age)) s")
              break
            }
          }
        }

        # The kill test: 5 batch engines of each kind killed from outside at seeded random times (90 to 240 s apart), each
        # once it has run 20 s; kinds alternate. A kind that does not show up for 15 minutes is swapped for the other.
        if ($mode -eq 'kill' -and $kills.Count -lt $killKinds.Count -and $startedTitles.Count -ge 1 -and ((Get-Date) - $lastKillProbe).TotalSeconds -ge 5 -and ($null -eq $nextKillAt -or (Get-Date) -ge $nextKillAt)) {
          $lastKillProbe = Get-Date
          $kind = $killKinds[$kills.Count]
          $tree = @(Get-WbProcessTree $emu.Id | ForEach-Object { [int]$_.ProcessId })
          $eng = @(Get-CimInstance Win32_Process -Filter "Name like 'wb-engine-$kind%'" -ErrorAction SilentlyContinue | Where-Object { $tree -contains [int]$_.ProcessId -and $_.CommandLine -match '\sbatch(\s|$)' })
          $hit = $null
          $hitAge = 0.0
          foreach ($e in $eng) {
            $age = ((Get-Date) - [datetime]$e.CreationDate).TotalSeconds
            if ($age -ge 20) { $hit = $e; $hitAge = $age; break }
          }
          if ($hit) {
            $kr = Invoke-WbNative -FilePath 'taskkill.exe' -ArgumentList @('/F', '/PID', "$($hit.ProcessId)")
            $kills += [pscustomobject]@{ N = $kills.Count + 1; At = (Get-Date).ToString('o'); Kind = $kind; Pid = $hit.ProcessId; AgeS = [math]::Round($hitAge, 1); Exit = $kr.ExitCode }
            Add-Content -LiteralPath (Join-Path $run 'script-events.log') -Value ("{0}`tKILL {1}: engine {2} pid {3} after {4:n0} s, taskkill exit {5}" -f (Get-Date -Format 'HH:mm:ss'), $kills.Count, $kind, $hit.ProcessId, $hitAge, $kr.ExitCode) -Encoding UTF8
            $progress.Add("$(Get-Date -Format 'HH:mm:ss') kill $($kills.Count) of $($killKinds.Count): $kind engine pid $($hit.ProcessId) after $([math]::Round($hitAge)) s")
            $nextKillAt = (Get-Date).AddSeconds(90 + $killRng.Next(0, 150))
            $killWaitSince = $null
          } else {
            if (-not $killWaitSince) { $killWaitSince = Get-Date }
            elseif (((Get-Date) - $killWaitSince).TotalMinutes -ge 15) {
              $killKinds[$kills.Count] = $(if ($kind -eq 'avx2') { 'vulkan' } else { 'avx2' })
              $killWaitSince = $null
              $progress.Add("$(Get-Date -Format 'HH:mm:ss') no $kind engine to kill for 15 minutes: the next kill is of the other kind")
            }
          }
        }

        if (-not $driverChange -and ((Get-Date) - $lastDriverCheck).TotalSeconds -ge 60) {
          $lastDriverCheck = Get-Date
          $driversNow = Get-WbGpuDrivers
          if ($driversNow -ne $driversStart) {
            $driverChange = "display drivers changed during the run: $driversStart -> $driversNow"
            $progress.Add("$(Get-Date -Format 'HH:mm:ss') ${driverChange}: the run is INVALID, stopping")
            if (-not (Test-Path -LiteralPath $stopFile)) { Set-Content -LiteralPath $stopFile -Value 'stop' -Encoding ascii }
          }
        }

        # End of the run: a short night pass, or the reason there is none.
        $stop = $false
        if ($mode -eq 'kill') {
          # Two recorded sessions and two imported copies, all four transcripts complete; or the hard limits below.
          if ($rollingCompleted -ge 4 -and -not $rollingAllAt) { $rollingAllAt = Get-Date; $progress.Add("$(Get-Date -Format 'HH:mm:ss') all four transcripts are complete ($($kills.Count) engines killed)") }
          if ($rollingAllAt -and ((Get-Date) - $rollingAllAt).TotalSeconds -ge 30) { $stop = $true }
        }
        if (-not $stop -and $mode -ne 'kill') {
          if ($nightDone -and ((Get-Date) - $nightDone).TotalSeconds -ge 30) { $stop = $true; $progress.Add("$(Get-Date -Format 'HH:mm:ss') the night pass is complete: stopping") }
          elseif ($nightPlanned -and ((Get-Date) - $nightPlanned).TotalMinutes -ge $NightMinutes) { $stop = $true; $progress.Add("$(Get-Date -Format 'HH:mm:ss') night pass time box of $NightMinutes min reached: stopping") }
          elseif ($nightSkipped -and ((Get-Date) - $nightSkipped).TotalSeconds -ge 30) { $stop = $true }
        }
        if (-not $stop -and $allRecorded -and -not $nightPlanned -and ((Get-Date) - $allRecorded).TotalMinutes -ge $DecisionMinutes) { $stop = $true; $progress.Add("$(Get-Date -Format 'HH:mm:ss') no night decision within $DecisionMinutes min of the last session: stopping") }
        if (-not $stop -and (Get-Date) -gt $hardLimit) { $stop = $true; $progress.Add("$(Get-Date -Format 'HH:mm:ss') hard limit of $MaxMinutes min: stopping") }
        if ($stop -and -not (Test-Path -LiteralPath $stopFile)) { Set-Content -LiteralPath $stopFile -Value 'stop' -Encoding ascii }

        if (((Get-Date) - $lastSample).TotalSeconds -ge 10) {
          $lastSample = Get-Date
          try { Add-ProcessSample -Path $sampleFile -RootId $emu.Id } catch { $progress.Add("$(Get-Date -Format 'HH:mm:ss') sampler: $($_.Exception.Message)") }
        }

        if (((Get-Date) - $lastProgress).TotalSeconds -ge 120) {
          $lastProgress = Get-Date
          $tail = @($lastLine)
          Write-Host "$(Get-Date -Format 'HH:mm:ss') running $([int]((Get-Date) - $started).TotalMinutes) min; sessions started $($startedTitles.Count); last log: $($tail -join ' ')"
        }
        Start-Sleep -Milliseconds 1000
      }
    } finally {
      foreach ($p in $players) { if ($p -and -not $p.HasExited) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue } }
      foreach ($r in $refs) {
        if ($r.Proc -and -not $r.Proc.HasExited) {
          if (-not (Test-Path -LiteralPath $r.Stop)) { Set-Content -LiteralPath $r.Stop -Value 'stop' -Encoding ascii }
          if (-not $r.Proc.WaitForExit(120000)) { Stop-Process -Id $r.Proc.Id -Force -ErrorAction SilentlyContinue }
        }
      }
      if ($emu -and -not $emu.HasExited) {
        Set-Content -LiteralPath $stopFile -Value 'stop' -Encoding ascii
        if (-not $emu.WaitForExit(120000)) { $null = Invoke-WbNative -FilePath 'taskkill.exe' -ArgumentList @('/F', '/T', '/PID', "$($emu.Id)") }
      }
      foreach ($v in 'WB_GATE_DAY_PLAN', 'WB_GATE_DAY_STOP_FILE', 'WB_GATE_REPORT', 'WB_GATE_BACKUP', 'WB_GATE_QUEUE_LOG', 'WB_GATE_CLOCK_OFFSET_S', 'WB_GATE_NIGHT_WAIT_S', 'WB_GATE_FORCE_HYBRID', 'WB_GATE_PART_SECONDS', 'WB_GATE_ROOT', 'WB_GATE_DATA_DIR', 'WB_GATE_CHECK') {
        Remove-Item "Env:\$v" -ErrorAction SilentlyContinue
      }
    }
    $driversEnd = Get-WbGpuDrivers
    if (-not $driverChange -and $driversEnd -ne $driversStart) { $driverChange = "display drivers changed during the run: $driversStart -> $driversEnd" }
    Write-Utf8 (Join-Path $run 'script-notes.json') ((@{
          drivers_start = $driversStart; drivers_end = $driversEnd; drivers_changed = [bool]$driverChange; driver_change = $driverChange
          started = $started.ToString('o'); ended = (Get-Date).ToString('o'); emu_exit = $emu.ExitCode; clock_offset_s = $offset
          mode = $mode; quiet = $quiet; kill = $kill; kills = @($kills); night_expected = $nightExpected; night_completed = $nightCompleted; progress = @($progress.ToArray()); profile = @{ tier = "$($d.tier)"; config = ($d.config.config | ConvertTo-Json -Compress); cpu_threads = $d.config.cpu_threads; process_while_recording = $d.process_while_recording }
          night_minutes = $NightMinutes; part_seconds = $PartSeconds; audio_dir = $Audio; cable = @{ play = $playName; capture = $capName }
        } | ConvertTo-Json -Depth 6))
    if ($fidelity) {
      Write-Host "$(Get-Date -Format 'HH:mm:ss') judging each session by the recording rule (analyse, compare)"
      $null = Invoke-FidelityJudging -Run $run -Refs $refs -ReportJson (Read-Json (Join-Path $run 'report.json'))
    }
    $run
  }
  $runDir = @($result)[-1]
  Write-SmokeEvidence $runDir
  $finalNotes = Read-Json (Join-Path $runDir 'script-notes.json')
  if ($finalNotes -and $finalNotes.drivers_changed) {
    Write-Host "RUN INVALID: $($finalNotes.driver_change)"
    exit 3
  }
} catch {
  $failed = $_
  Write-Host "FAIL: the smoke run did not complete: $($_.Exception.Message)"
  Write-Host $_.ScriptStackTrace
  exit 1
}
