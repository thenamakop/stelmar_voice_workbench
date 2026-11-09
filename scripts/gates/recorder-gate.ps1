<#
.SYNOPSIS
  Phase 1 recorder gates on the real app (gate build): soak, load, panic and webview.

.DESCRIPTION
  Runs the gate build of the app (target\gate, cargo feature gate-hooks; never an installer
  build) inside an emulation job (wb-emulate, default emu-low), records from the virtual cable
  while wb-testbench plays a known noise file into it from outside the job, stops the session,
  then checks the recording against the source.

    soak     2 hours (default): duration within 1 s of the source, no gaps, zero overruns,
             recorder CPU under 5%, mirror SHA-256 equal.
    load     30 minutes with every logical CPU of the job busy at normal priority and a compute
             load on the integrated GPU, both inside the same job: zero overruns, no gaps.
    panic    3 minutes; after 60 s the app panics once inside the command boundary and once on a
             plain thread: recording continues with no gap.
    webview  3 minutes; after 40 s the WebView2 browser process is killed: the display recovers
             in a new window and the recording has no gap.

  Every run: RTSS and MSI Afterburner must be closed, D:\Projects\stelmar-gate.lock is held for
  the run (Track B pauses), and the evidence file goes through scripts/lib/Redact.psm1. Audio
  stays under -WorkRoot and -BackupRoot, never in the repository.

  Needs: VB-CABLE, the gate build (scripts\gates\build-gate.ps1), wb-testbench and wb-emulate
  release builds. No administrator rights.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File .\scripts\gates\recorder-gate.ps1 -Gate soak
  powershell -ExecutionPolicy Bypass -File .\scripts\gates\recorder-gate.ps1 -Gate load -Profile emu-low
#>
[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)] [ValidateSet('soak', 'load', 'panic', 'webview')] [string]$Gate,
  [ValidateSet('emu-low', 'emu-mid', 'native')] [string]$Profile = 'emu-low',
  [double]$Minutes = 0,
  [string]$Machine = 'Legion',
  [string]$WorkRoot = 'D:\STELMAR-Gate',
  [string]$BackupRoot = 'C:\STELMAR-Gate-Backup',
  [string]$CaptureDevice = 'CABLE Output',
  [string]$PlayDevice = 'CABLE Input',
  [string]$GpuName = 'UHD',
  [string]$OutDir,
  [string]$Exe,
  [string]$Testbench,
  [string]$Emulate,
  # Re-evaluate a run that already happened (for example after a harness crash) from its saved
  # recording and reports in this folder, instead of running the gate again. The original run's
  # start time and free RAM come from D:\STELMAR-Gate\run-all.log; pass them here.
  # Run even though the cable is a default playback device (every sound the machine makes then
  # goes into the test signal). The evidence says so.
  [switch]$AllowCableDefault,
  [string]$FromRun,
  [string]$OriginalStarted,
  [double]$OriginalFreeRamStart = 0,
  [double]$OriginalFreeRamEnd = 0,
  [string]$OriginalNote = ''
)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo = Resolve-Path (Join-Path $here '..\..')
if (-not $OutDir) { $OutDir = Join-Path $repo 'docs\evidence\phase-1' }
if (-not $Exe) { $Exe = Join-Path $repo 'target\gate\stelmar-voice-workbench.exe' }
if (-not $Testbench) { $Testbench = Join-Path $repo 'target\release\wb-testbench.exe' }
if (-not $Emulate) { $Emulate = Join-Path $repo 'target\release\wb-emulate.exe' }
if ($Minutes -le 0) { $Minutes = @{ soak = 120; load = 30; panic = 3; webview = 3 }[$Gate] }
Import-Module (Join-Path $repo 'scripts\lib\PsEnv.psm1') -Force
Set-WbCleanPSModulePath
Import-Module (Join-Path $repo 'scripts\lib\GateRun.psm1') -Force
Import-Module (Join-Path $repo 'scripts\lib\Redact.psm1') -Force
Import-Module (Join-Path $repo 'scripts\lib\AppProcess.psm1') -Force
Import-Module (Join-Path $repo 'scripts\lib\Rig.psm1') -Force
Import-Module (Join-Path $repo 'scripts\lib\Fidelity.psm1') -Force
foreach ($f in @($Exe, $Testbench, $Emulate)) { if (-not (Test-Path $f)) { throw "Missing $f; build it first (scripts\gates\build-gate.ps1)" } }

function Q([string]$v) { ConvertTo-WbArg $v }

function Get-Descendants([int]$RootId) {
  @(Get-WbProcessTree $RootId | ForEach-Object { [int]$_.ProcessId })
}

function Get-NvidiaPids {
  $out = (Invoke-WbNative -FilePath 'nvidia-smi').Output
  $pids = @()
  foreach ($l in $out) {
    if ($l -match '^\|\s+\S+\s+\S+\s+\S+\s+(\d+)\s+(C\+G|C|G)\s') { $pids += [int]$Matches[1] }
  }
  $pids
}

# Audio sessions playing into the cable now, other than $Allow (process ids). Anything else
# mixes into the test signal (seen in Phase 1: a browser tab routed to CABLE Input).
function Get-CableIntruders([int[]]$Allow = @()) {
  $res = Invoke-WbNative -FilePath $Testbench -ArgumentList @('sessions', '--device', $PlayDevice)
  if ($res.ExitCode -ne 0 -or $res.Output.Count -eq 0) { return @() }
  @((($res.Output -join "`n") | ConvertFrom-Json) | Where-Object { $_.active -and $Allow -notcontains [int]$_.pid } | ForEach-Object {
    $name = (Get-Process -Id $_.pid -ErrorAction SilentlyContinue).ProcessName
    "$name (pid $($_.pid)) on $($_.endpoint)"
  })
}

function Read-Json([string]$Path) {
  if (Test-Path -LiteralPath $Path) { Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json } else { $null }
}

$logFile = Join-Path $env:LOCALAPPDATA 'in.edu.bmu.stelmar.voiceworkbench\logs\app.log'

# Builds a gate result from a run that already happened (-FromRun): the saved recording and JSON
# reports, with the analysis done again by the current analyser. Things measured live and not
# saved (the NVIDIA-GPU check, the cable-interference polling, the app log) are $null here and are
# reported as not re-evaluated.
function Get-ResultFromRun([string]$run) {
  $sessionDir = (Get-ChildItem (Join-Path $run 'rec') -Directory | ForEach-Object { Get-ChildItem $_.FullName -Directory } | Select-Object -First 1).FullName
  $play = Read-Json (Join-Path $run 'play.json')
  $source = Join-Path (Join-Path $WorkRoot 'source') $play.file
  $refDir = Join-Path $run 'reference'
  $analysisErr = Invoke-WbAnalyse -Testbench $Testbench -Session $sessionDir -Source $source -Report (Join-Path $run 'analysis.json')
  $refAnalysisErr = Invoke-WbAnalyse -Testbench $Testbench -Session $refDir -Source $source -Report (Join-Path $run 'reference-analysis.json')
  & $Testbench decode --session $sessionDir --report (Join-Path $run 'decode.json') | Out-Null
  $cmp = Invoke-WbNative -FilePath $Testbench -ArgumentList @('compare', '--app', $sessionDir, '--reference', $refDir, '--report', (Join-Path $run 'compare.json'))
  [pscustomobject]@{
    Run = $run; Compare = (Read-Json (Join-Path $run 'compare.json')); CompareError = $(if ($cmp.ExitCode -ne 0) { (@($cmp.Error) + @($cmp.Output)) -join ' ' } else { $null }); SessionDir = $sessionDir; Source = $source; Report = (Read-Json (Join-Path $run 'stop-report.json')); Inner = (Read-Json (Join-Path $run 'inner.json'))
    Play = $play; Stress = (Read-Json (Join-Path $run 'stress.json')); Analysis = (Read-Json (Join-Path $run 'analysis.json')); Decode = (Read-Json (Join-Path $run 'decode.json'))
    Progress = @("re-evaluated from the saved run $run"); CableDefaultRoles = $null; AppLog = @(); NvOverlap = $null; JobPids = $null; NvPids = $null; WebviewKill = $null; Interference = $null
    LastStatus = (Read-Json (Join-Path $run 'status.json')); HostStress = (Read-Json (Join-Path $run 'host-stress.json'))
    Reference = (Read-Json (Join-Path $run 'reference.json')); RefAnalysis = (Read-Json (Join-Path $run 'reference-analysis.json'))
    AnalysisError = $analysisErr; RefAnalysisError = $refAnalysisErr; Rig = $null
  }
}

$result = $null
try {
if ($FromRun) {
  if (-not $OriginalStarted) { throw '-FromRun needs -OriginalStarted (when the original run started)' }
  Set-WbGateContext -Name "phase1-$Gate-$Profile (re-evaluated from saved data)" -Started ([datetime]::Parse($OriginalStarted)) -FreeRamStart $OriginalFreeRamStart -FreeRamEnd $OriginalFreeRamEnd -Note "evaluation redone from the saved recording and reports of the run in $FromRun with the current analyser; $OriginalNote"
  $result = Get-ResultFromRun $FromRun
} else {
$result = Invoke-WbGate -Name "phase1-$Gate-$Profile" -Body {
  if (Get-Process stelmar-voice-workbench -ErrorAction SilentlyContinue) { throw 'The app is already running; close it first.' }
  $cableRoles = @(Get-WbCableDefaultRoles -Testbench $Testbench -Cable $PlayDevice)
  if ($cableRoles.Count -gt 0 -and -not $AllowCableDefault) { throw ("The default playback device ($($cableRoles -join ', ')) is ${PlayDevice}: every sound this computer makes would play into the test signal. Set the default playback device to the speakers (Windows Settings > System > Sound), then run the gate again (or pass -AllowCableDefault).") }
  $intruders = @(Get-CableIntruders)
  if ($intruders.Count -gt 0) { throw ("Another program is playing into $PlayDevice : " + ($intruders -join ', ') + '. Stop it (or close it) so only the gate player feeds the cable.') }
  # Where the player and the reference capture will run: outside the job's cores. The gate does
  # not start if the plan overlaps the job or itself.
  $rigPlan = Get-WbRigPlan -Emulate $Emulate -Profile $Profile
  if ($rigPlan) { Assert-WbRigPlacement -JobMask $rigPlan.JobMask -PlayerMask $rigPlan.PlayerMask -ReferenceMask $rigPlan.ReferenceMask -What 'planned, before anything starts' }
  $interference = New-Object System.Collections.Generic.List[string]
  $stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
  $run = Join-Path $WorkRoot "$Gate-$Profile-$stamp"
  $rec = Join-Path $run 'rec'
  $backup = Join-Path $BackupRoot "$Gate-$Profile-$stamp"
  New-Item -ItemType Directory -Force -Path $rec | Out-Null
  $srcDir = Join-Path $WorkRoot 'source'
  New-Item -ItemType Directory -Force -Path $srcDir | Out-Null
  $seconds = [int][math]::Round($Minutes * 60)
  $source = Join-Path $srcDir "noise-${seconds}s.wav"
  if (-not (Test-Path ($source + '.json'))) {
    & $Testbench gen --out $source --seconds $seconds --rate 48000 --channels 2 | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'wb-testbench gen failed' }
  }
  $stopFile = Join-Path $run 'STOP'
  $stressStop = Join-Path $run 'STOP-STRESS'
  $env:WB_GATE_ROOT = $rec
  $env:WB_GATE_BACKUP = $backup
  $env:WB_GATE_START = "Gate $Gate $stamp"
  $env:WB_GATE_DEVICE = $CaptureDevice
  $env:WB_GATE_STOP_FILE = $stopFile
  $env:WB_GATE_REPORT = Join-Path $run 'stop-report.json'
  $env:WB_GATE_STATUS_FILE = Join-Path $run 'status.json'
  $env:WB_GATE_RECOVERY = ''
  $env:WB_GATE_PANIC_AT_S = $(if ($Gate -eq 'panic') { '60' } else { '' })
  $env:WB_GATE_RECORDER_PANIC_AT_S = $(if ($Gate -eq 'panic') { '100' } else { '' })
  $env:WB_GATE_PART_SECONDS = ''
  $logStart = $(if (Test-Path $logFile) { (Get-Item $logFile).Length } else { 0 })

  $inner = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', (Q (Join-Path $here 'recorder-gate-inner.ps1')),
    '-Exe', (Q $Exe), '-ResultFile', (Q (Join-Path $run 'inner.json')))
  if ($Gate -eq 'load') {
    $inner += @('-Stress', '-Testbench', (Q $Testbench), '-GpuName', (Q $GpuName), '-StressStopFile', (Q $stressStop), '-StressReport', (Q (Join-Path $run 'stress.json')))
  }
  $emuArgs = @('--profile', $Profile, '--log', (Q (Join-Path $run 'emulate.log')), '--', 'powershell.exe') + $inner
  $emu = Start-Process -FilePath $Emulate -ArgumentList $emuArgs -PassThru -WindowStyle Hidden

  # Wait until the session records.
  $deadline = (Get-Date).AddSeconds(120)
  while ($true) {
    $st = Read-Json $env:WB_GATE_STATUS_FILE
    if ($st -and $st.recorder -and $st.recorder.state -eq 'recording') { break }
    if ($emu.HasExited) { throw "The job ended before recording started (exit $($emu.ExitCode)); see $logFile" }
    if ((Get-Date) -gt $deadline) { throw 'Recording did not start within 120 s' }
    Start-Sleep -Milliseconds 500
  }
  $sessionDir = $st.recorder.session_dir
  $progress = New-Object System.Collections.Generic.List[string]
  $progress.Add("$(Get-Date -Format 'HH:mm:ss') recording in $sessionDir")

  # Load test: also load every logical processor of the host outside the job, at normal priority,
  # so the audio engine and the cable driver (which run outside the job) are contended too.
  $hostStress = $null
  if ($Gate -eq 'load') {
    $hostStress = Start-Process -FilePath $Testbench -ArgumentList @('stress', '--threads', "$([Environment]::ProcessorCount)", '--seconds', '14400', '--stop-file', (Q $stressStop), '--report', (Q (Join-Path $run 'host-stress.json'))) -PassThru -WindowStyle Hidden
  }
  # Independent reference capture of the same cable, outside the job (wb-testbench capture).
  $refDir = Join-Path $run 'reference'
  $refStop = Join-Path $run 'STOP-REF'
  $goFile = Join-Path $run 'GO-RIG'
  $refProc = Start-Process -FilePath $Testbench -ArgumentList @('capture', '--device', (Q $CaptureDevice), '--dir', (Q $refDir), '--stop-file', (Q $refStop), '--go-file', (Q $goFile), '--report', (Q (Join-Path $run 'reference.json'))) -PassThru -WindowStyle Hidden
  Start-Sleep -Seconds 1
  $play = Start-Process -FilePath $Testbench -ArgumentList @('play', '--device', (Q $PlayDevice), '--file', (Q $source), '--go-file', (Q $goFile), '--report', (Q (Join-Path $run 'play.json'))) -PassThru -WindowStyle Hidden
  # Pin both to cores outside the job at HIGH priority class; they wait for the go file before
  # they open their streams, so no callback runs unpinned. Then check what they really have.
  $rig = Set-WbRigPins -Plan $rigPlan -Player $play -Reference $refProc -GoFile $goFile
  $rig = Confirm-WbRigAgainstJob -Rig $rig -Player $play -Reference $refProc
  if ($rig) { $progress.Add("$(Get-Date -Format 'HH:mm:ss') rig pinned: job $(Format-WbMask $rig.Job.Mask), player $(Format-WbMask $rig.Player.Mask), reference capture $(Format-WbMask $rig.Reference.Mask), no overlap") }

  # GPU visibility: no process of the job may appear in nvidia-smi.
  Start-Sleep -Seconds 15
  $jobPids = @(Get-Descendants $emu.Id)
  $nvPids = @(Get-NvidiaPids)
  $nvOverlap = @($nvPids | Where-Object { $jobPids -contains $_ })
  $progress.Add("$(Get-Date -Format 'HH:mm:ss') job processes: $($jobPids.Count); nvidia-smi processes: $($nvPids.Count); job processes on the NVIDIA GPU: $($nvOverlap.Count)")

  $webviewKill = $null
  if ($Gate -eq 'webview') {
    Start-Sleep -Seconds 25
    $browser = @(Get-WbWebView2 | Where-Object { $_.Type -eq 'browser' })
    if ($browser.Count -eq 0) { throw 'No WebView2 browser process found' }
    $at = Get-Date
    Stop-Process -Id $browser[0].ProcessId -Force
    $webviewKill = "killed WebView2 browser process $($browser[0].ProcessId) at $($at.ToString('HH:mm:ss.fff'))"
    $progress.Add("$($at.ToString('HH:mm:ss')) $webviewKill")
  }

  $lastProgress = Get-Date
  $lastCable = Get-Date
  while (-not $play.HasExited) {
    if (((Get-Date) - $lastCable).TotalSeconds -ge 1) {
      $lastCable = Get-Date
      foreach ($i in (Get-CableIntruders -Allow @($play.Id))) { $interference.Add("$(Get-Date -Format 'HH:mm:ss') $i") }
    }
    if (((Get-Date) - $lastProgress).TotalSeconds -ge 60) {
      $lastProgress = Get-Date
      $s = Read-Json $env:WB_GATE_STATUS_FILE
      if ($s -and $s.recorder) {
        $r = $s.recorder
        $lag = $(if ($s.mirror) { "$($s.mirror.state) lag $($s.mirror.lag_ms) ms" } else { 'mirror ?' })
        $progress.Add(("{0} audio {1:n1} min, part {2}, overrun samples {3}, xruns {4}, ring max {5:n2}%, recorder cpu {6} ms, {7}" -f (Get-Date -Format 'HH:mm:ss'), ($r.audio_ms / 60000.0), $r.part, $r.overrun_samples, $r.xruns, $r.ring_max_fill_pct, $r.thread_cpu_ms, $lag))
      }
      if ($emu.HasExited) { throw 'The app exited during the run' }
    }
    Start-Sleep -Seconds 1
  }
  Start-Sleep -Seconds 2
  $lastStatus = Read-Json $env:WB_GATE_STATUS_FILE
  Set-Content -LiteralPath $refStop -Value 'stop'
  [void]$refProc.WaitForExit(120000)
  Set-Content -LiteralPath $stopFile -Value 'stop'
  if ($hostStress) { Set-Content -LiteralPath $stressStop -Value 'stop'; [void]$hostStress.WaitForExit(60000) }
  if (-not $emu.WaitForExit(900000)) { throw 'The app did not stop within 15 minutes' }

  $report = Read-Json $env:WB_GATE_REPORT
  $innerRes = Read-Json (Join-Path $run 'inner.json')
  $playRes = Read-Json (Join-Path $run 'play.json')
  $stressRes = Read-Json (Join-Path $run 'stress.json')
  $analysisErr = Invoke-WbAnalyse -Testbench $Testbench -Session $sessionDir -Source $source -Report (Join-Path $run 'analysis.json')
  $analysis = Read-Json (Join-Path $run 'analysis.json')
  & $Testbench decode --session $sessionDir --report (Join-Path $run 'decode.json') | Out-Null
  $refAnalysisErr = Invoke-WbAnalyse -Testbench $Testbench -Session $refDir -Source $source -Report (Join-Path $run 'reference-analysis.json')
  $decode = Read-Json (Join-Path $run 'decode.json')
  $cmp = Invoke-WbNative -FilePath $Testbench -ArgumentList @('compare', '--app', $sessionDir, '--reference', $refDir, '--report', (Join-Path $run 'compare.json'))
  $appLog = @()
  if (Test-Path $logFile) {
    $fs = [System.IO.File]::Open($logFile, 'Open', 'Read', 'ReadWrite')
    try { [void]$fs.Seek($logStart, 'Begin'); $sr = New-Object System.IO.StreamReader($fs); $appLog = $sr.ReadToEnd() -split "`r?`n" } finally { $fs.Dispose() }
  }
  [pscustomobject]@{
    Run = $run; SessionDir = $sessionDir; Source = $source; Report = $report; Inner = $innerRes; Play = $playRes
    Compare = (Read-Json (Join-Path $run 'compare.json')); CompareError = $(if ($cmp.ExitCode -ne 0) { (@($cmp.Error) + @($cmp.Output)) -join ' ' } else { $null })
    Stress = $stressRes; Analysis = $analysis; Decode = $decode; Progress = $progress; AppLog = $appLog
    NvOverlap = $nvOverlap; JobPids = $jobPids; NvPids = $nvPids; WebviewKill = $webviewKill; Interference = @(ConvertTo-WbArray $interference)
    LastStatus = $lastStatus; HostStress = (Read-Json (Join-Path $run 'host-stress.json'))
    Reference = (Read-Json (Join-Path $run 'reference.json')); RefAnalysis = (Read-Json (Join-Path $run 'reference-analysis.json'))
    AnalysisError = $analysisErr; RefAnalysisError = $refAnalysisErr; CableDefaultRoles = $cableRoles; Rig = $rig
  }
}
}
} catch {
  $detail = $(if ($script:Current -and $script:Current.Error) { $script:Current.Error } else { ($_.Exception.GetType().FullName + ': ' + $_.Exception.Message + "`r`n" + $_.ScriptStackTrace) })
  Write-WbGateCrashEvidence -Path (Join-Path $OutDir "$Gate-$Profile.txt") -Reason "the $Gate gate stopped on an error: $($_.Exception.Message)" -Detail $detail -Machine $Machine -Profile $Profile -Builds @($Exe, $Testbench, $Emulate)
  Write-Host "FAIL: the $Gate gate did not complete: $($_.Exception.Message)"
  exit 1
}

# ---- Verdict and evidence ----
$r = @($result)[-1]
$missing = @(Test-WbGateData -Result $r -Required @('Report', 'Inner', 'Play', 'Analysis', 'Decode', 'Reference', 'RefAnalysis', 'LastStatus'))
if ($missing.Count -eq 0 -and -not ((Test-WbHasProperty $r.Report 'stopped') -and $null -ne $r.Report.stopped)) { $missing = @('Report.stopped (the app did not report a stop)') }
if ($missing.Count -gt 0) {
  $extra = @("missing: $($missing -join ', ')")
  if ($r.AnalysisError) { $extra += "analyser (app recording): $($r.AnalysisError)" }
  if ($r.RefAnalysisError) { $extra += "analyser (reference capture): $($r.RefAnalysisError)" }
  $extra += "run folder: $($r.Run) (kept; nothing was deleted)"
  Write-WbGateCrashEvidence -Path (Join-Path $OutDir "$Gate-$Profile.txt") -Reason 'no verdict data: the run produced no analysis or reports to judge' -Machine $Machine -Profile $Profile -Builds @($Exe, $Testbench, $Emulate) -Extra $extra
  Write-Host "FAIL: no verdict data ($($missing -join ', '))"
  exit 1
}
$stopped = $r.Report.stopped
$rec = $stopped.recorder
$logical = 4
$emuLog = Join-Path $r.Run 'emulate.log'
if (Test-Path $emuLog) { $m = Select-String -Path $emuLog -Pattern 'logical_processors=(\d+)' | Select-Object -First 1; if ($m) { $logical = [int]$m.Matches[0].Groups[1].Value } }
$wall = [double]$r.Inner.app_wall_seconds
$appCpuPctMachine = 100.0 * [double]$r.Inner.app_cpu_seconds / ($wall * $logical)
$appCpuPctCore = 100.0 * [double]$r.Inner.app_cpu_seconds / $wall
$recCpuPctCore = 100.0 * ([double]$rec.recorder_cpu_ms / 1000.0) / $wall
$checks = [ordered]@{}
$split = Split-WbDiscontinuities -App $r.Analysis -Reference $r.RefAnalysis
$refUnmatched = $(if ($r.RefAnalysis) { [double]$r.RefAnalysis.unmatched_ms } else { 0 })
# The rule of 9 Nov 2026 (docs/PLAN.md section 12): the app is judged against the reference
# capture (what Windows delivered), not against the source.
$playStart = [DateTimeOffset]::Parse($r.Play.started_at)
$rv = Get-WbRunVerdict -Compare $r.Compare -RefAnalysis $r.RefAnalysis -Play $r.Play -Reference $r.Reference -PlayStarted $playStart -DroppedSamples ([int64]$stopped.recorder.dropped_samples)
$fidelityText = $(if ($r.Compare) { "differing frames $($r.Compare.differing_frames), reference audio without an app recording $($r.Compare.reference_only_loud_frames) frames" } else { 'no comparison data' })
$checks['reference capture ran cleanly (its own ring overruns 0)'] = ($r.Reference -and [int64]$r.Reference.overrun_samples -eq 0 -and $r.RefAnalysis)
$checks["fidelity: the app's recording equals the reference capture sample for sample, one fixed offset per run ($fidelityText)"] = ($null -ne $r.Compare -and [bool]$r.Compare.aligned -and [bool]$r.Compare.equal)
$checks['recorded duration within 1 s of the source'] = ([math]::Abs([double]$r.Analysis.duration_diff_ms) -le 1000)
$unexplained = Get-WbUnexplainedUnmatchedMs -App $r.Analysis -Near $split.Excused
if ($null -ne $r.Interference) { $checks['no other program played into the cable'] = (@($r.Interference).Count -eq 0) }
$checks['zero ring-buffer overruns'] = ([int64]$rec.overrun_samples -eq 0)
$lossFree = ($null -ne $r.Compare -and [bool]$r.Compare.equal)
$xr = Get-WbXrunVerdict -SessionDir $r.SessionDir -LossFree $lossFree
$checks["driver xruns: none, or one within 2 s of the stream start with zero loss around it (xruns $($rec.xruns), excused $(@($xr.Excused).Count))"] = ([int64]$rec.xruns -eq 0 -or $xr.Pass)
$checks['zero samples dropped by the writer'] = ([int64]$rec.dropped_samples -eq 0)
$checks["capture thread at MMCSS priority ($($r.LastStatus.recorder.capture_priority))"] = ("$($r.LastStatus.recorder.capture_priority)" -eq 'mmcss')
if ($Gate -ne 'panic') { $checks['no recorder restarts'] = ([int]$rec.recorder_restarts -eq 0) }
$checks['every part decodes in Media Foundation'] = [bool]$r.Decode.all_playable
$checks['mirror SHA-256 equals the originals'] = [bool]$stopped.mirror.all_match
$checks['player had no underruns (source played cleanly)'] = ([int64]$r.Play.underrun_callbacks -eq 0)
if ($null -ne $r.NvOverlap) { $checks['no job process on the NVIDIA GPU'] = (@($r.NvOverlap).Count -eq 0) }
if ($Gate -eq 'soak') { $checks['app process CPU under 5% of the job''s processors'] = ($appCpuPctMachine -lt 5.0) }
if ($Gate -eq 'load') {
  $checks['stress: every logical CPU of the job busy'] = ($r.Stress -and [int]$r.Stress.cpu_threads -eq $logical)
  $checks['stress: GPU compute load ran on the integrated GPU'] = ($r.Stress -and [int64]$r.Stress.gpu_dispatches -gt 0 -and "$($r.Stress.gpu_adapter)" -match $GpuName)
  $checks["host stress: all $([Environment]::ProcessorCount) logical processors busy outside the job too"] = ($r.HostStress -and [int]$r.HostStress.cpu_threads -eq [Environment]::ProcessorCount)
}
if ($Gate -eq 'panic') {
  $pi = $r.Report.panic_injection
  $checks['panic in the command boundary returned an error'] = ($pi -and "$($pi.boundary_result)" -match '^Err')
  $checks['panic on a plain thread happened'] = ($pi -and [bool]$pi.thread_panicked)
  $checks['panic inside the recorder thread: it restarted once and continued in a new part'] = ([bool]$r.Report.recorder_panic_injection.sent -and [int]$rec.recorder_restarts -eq 1 -and @($rec.parts).Count -ge 2)
}
if ($Gate -eq 'webview' -and @($r.AppLog).Count -gt 0) {
  $checks['display recovered in a new window'] = [bool]($r.AppLog | Where-Object { $_ -match 'display recovered in a new window' })
}
$pass = -not ($checks.Values -contains $false) -and $rv.Verdict -ne 'FAIL'
$verdict = $(if ($rv.Verdict -eq 'INVALID') { 'INVALID' } elseif ($pass) { 'PASS' } else { 'FAIL' })
$lines_cable = $(if ($null -eq $r.Interference) { 'cable interference polling: not re-evaluated (measured live, not saved); the analysis above is the evidence' } elseif (@($r.Interference).Count -gt 0) { 'cable interference seen (run invalid for the "equals the source" check): ' + (@($r.Interference) -join '; ') } else { 'cable interference: none seen by the 1-second session poll (sounds shorter than a second can be missed; the analysis above is the check)' })

$lines = @()
$lines += Get-WbGateHeader -Machine $Machine -Profile $Profile -Builds @($Exe, $Testbench, $Emulate)
$lines += "app build: target\gate\stelmar-voice-workbench.exe (cargo profile gate, features gate-hooks + custom-protocol)"
$lines += "verdict: $(if ($verdict -eq 'INVALID') { 'INVALID (no verdict; the run must be repeated)' } elseif ($verdict -eq 'PASS' -and $Profile -ne 'native') { 'PASS (PROVISIONAL: emulated)' } else { $verdict })"
$lines += ''
$lines += 'checks:'
foreach ($k in $checks.Keys) { $lines += "  [$(if ($checks[$k]) { 'x' } else { ' ' })] $k" }
$lines += ''
$lines += Get-WbVerdictLines -Result $rv -Compare $r.Compare
if ($r.CompareError) { $lines += "comparison error: $($r.CompareError)" }
if ($Gate -eq 'webview' -and @($r.AppLog).Count -eq 0) { $lines += 'display recovery: not re-evaluated here (the live app log is not saved); the original run''s evidence stands' }
$lines += ''
$lines += 'measurements:'
$lines += ("  source: {0:n1} min of known noise, 48 kHz stereo, played by wb-testbench into '{1}' from outside the job" -f ($r.Analysis.source_ms / 60000.0), $PlayDevice)
$lines += ("  recorded content: {0:n3} s against source {1:n3} s (difference {2:n1} ms); discontinuities {3}; silent gaps {4:n1} ms; unmatched {5:n1} ms; lowest block correlation {6:n4}" -f ($r.Analysis.content_ms / 1000.0), ($r.Analysis.source_ms / 1000.0), $r.Analysis.duration_diff_ms, @($r.Analysis.discontinuities).Count, $r.Analysis.silent_gap_ms, $r.Analysis.unmatched_ms, $r.Analysis.min_block_correlation)
$lines += ("  parts: {0}; audio {1:n1} s; overrun samples {2}; driver xruns {3}; samples dropped by the writer {4}; recorder restarts {5}" -f @($rec.parts).Count, ($rec.audio_ms / 1000.0), $rec.overrun_samples, $rec.xruns, $rec.dropped_samples, $rec.recorder_restarts)
$lines += ("  priorities: recorder thread {0}, capture thread {1}; ring buffer {2} s, highest fill {3:n2}%" -f $r.LastStatus.recorder.priority, $r.LastStatus.recorder.capture_priority, $r.LastStatus.recorder.ring_seconds, $r.LastStatus.recorder.ring_max_fill_pct)
$lines += ("  recorder thread CPU: {0:n2} s over {1:n0} s = {2:n3}% of one core" -f ($rec.recorder_cpu_ms / 1000.0), $wall, $recCpuPctCore)
$lines += ("  app process CPU (main process; WebView2 processes not counted): {0:n1} s = {1:n2}% of the job's {2} logical processors ({3:n2}% of one core)" -f $r.Inner.app_cpu_seconds, $appCpuPctMachine, $logical, $appCpuPctCore)
$lines += ("  mirror: {0} files compared, all equal: {1} (mirror on the same machine, separate drive pending: C: instead of an external SSD)" -f @($stopped.mirror.files).Count, $stopped.mirror.all_match)
$lines += ("  player: {0} frames of {1}, underrun callbacks {2}, render thread priority {3}, output buffer {4}" -f $r.Play.frames_played, $r.Play.frames_in_file, $r.Play.underrun_callbacks, $r.Play.render_priority, $r.Play.buffer)
$lines += Get-WbRigLines -Rig $r.Rig -Play $r.Play -Reference $r.Reference
$breakSources = @(@($r.Analysis.unmatched_ranges) | ForEach-Object { [double]$_.source_ms }) + @(@($r.Analysis.discontinuities) | ForEach-Object { [double]$_.source_ms })
$lines += Get-WbLateCallbackLines -Play $r.Play -BreakSourceMs @($breakSources | Sort-Object -Unique)
$lines += "  $lines_cable"
$lines += $(if ($null -eq $r.CableDefaultRoles) { '  default playback device: not recorded for this run' } elseif (@($r.CableDefaultRoles).Count -gt 0) { "  default playback device: THE CABLE for $(@($r.CableDefaultRoles) -join ', ') (run with -AllowCableDefault: other sounds on this computer can reach the test signal)" } else { '  default playback device: not the cable (checked at start)' })
$playStart = [DateTimeOffset]::Parse($r.Play.started_at)
$lines += ("  signal starts at source frame {0} ({1:n1} ms of the source's start never reached the captures: playback start-up); leading audio before the signal: {2:n0} ms" -f $r.Analysis.content_first_source_frame, ($r.Analysis.content_first_source_frame / 48.0), $r.Analysis.leading_junk_ms)
foreach ($u in @($r.Analysis.unmatched_ranges)) {
  $alsoRef = @($r.RefAnalysis.unmatched_ranges | Where-Object { [math]::Abs([double]$_.source_ms - [double]$u.source_ms) -le 100 }).Count -gt 0
  $when = $playStart.AddMilliseconds([double]$u.source_ms - [double]$r.Analysis.content_first_source_frame / 48.0)
  $lines += ("  unmatched stretch: source {0:n2} s (about {1} IST), {2:n0} ms; also in the reference capture: {3}" -f ([double]$u.source_ms / 1000.0), $when.ToString('HH:mm:ss'), [double]$u.ms, $alsoRef)
}
$lines += Get-WbReferenceXrunLines -RefAnalysis $r.RefAnalysis -Reference $r.Reference -PlayStarted $playStart -BreakSourceMs @(@($r.Analysis.unmatched_ranges) | ForEach-Object { [double]$_.source_ms })
$lines += ("  reference capture (separate cpal + hound recorder outside the job; shares the audio engine, cable driver and cpal with the app): {0} breaks, unmatched {1} ms, its overruns {2}, xruns {3}" -f @($r.RefAnalysis.discontinuities).Count, $refUnmatched, $r.Reference.overrun_samples, $r.Reference.xruns)
$lines += ("  breaks in the app recording: recorder's own {0}; shared with the reference (upstream; see the upstream breaks section) {1}; start-up repeats within the old allowance {2} {3}" -f $split.Own.Count, $split.Chain.Count, $split.Excused.Count, ((@($split.Excused) | ForEach-Object { "(at source $([math]::Round([double]$_.source_ms)) ms, $([math]::Round([double]$_.source_jump_ms, 1)) ms)" }) -join ' '))
$lines += ("  capture position breaks counted in the app's callback (device time not following on): {0}" -f $rec.position_breaks)
$lines += $(if ($null -ne $r.NvOverlap) { "  GPU check: {0} job processes, {1} processes in nvidia-smi, overlap {2}" -f @($r.JobPids).Count, @($r.NvPids).Count, @($r.NvOverlap).Count } else { '  GPU check: not re-evaluated (measured live, not saved); emulator-check.txt covers it' })
if ($r.Stress) { $lines += ("  stress: {0} threads at {1:n1} M loops/thread/s for {2:n0} s; GPU {3}: {4} dispatches {5}" -f $r.Stress.cpu_threads, $r.Stress.cpu_mloops_per_thread_s, $r.Stress.seconds, $r.Stress.gpu_adapter, $r.Stress.gpu_dispatches, $r.Stress.gpu_error) }
if ($r.WebviewKill) { $lines += "  webview: $($r.WebviewKill); display restarts reported: $($r.Report.display_restarts)" }
if ($Gate -eq 'panic') { $lines += "  panic injection: $($r.Report.panic_injection | ConvertTo-Json -Compress)" }
$lines += ''
$lines += 'progress:'
$lines += ($r.Progress | ForEach-Object { "  $_" })
$lines += ''
$lines += 'emulation log:'
if (Test-Path $emuLog) { $lines += (Get-Content $emuLog | ForEach-Object { "  $_" }) }
$lines += ''
$lines += 'discontinuities:'
$lines += (@($r.Analysis.discontinuities) | ForEach-Object { "  $($_ | ConvertTo-Json -Compress)" })
$lines += ''
$lines += 'decode (Media Foundation source reader, the decoder Windows players use):'
$lines += (@($r.Decode.parts) | ForEach-Object { "  $($_.file): header $($_.header_ms) ms, decoded $($_.decoded_ms) ms, playable $($_.playable)" })
$lines += ''
$lines += 'app log for this run (warnings, errors, session lines):'
$lines += (@($r.AppLog) | Where-Object { $_ -match 'WARN|ERROR|session|part closed|recovery|display|gate|panic' } | Select-Object -Last 80 | ForEach-Object { "  $_" })
$lines += ''
$lines += "audio and reports: $($r.Run) (not in the repository)"

New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$evidence = Join-Path $OutDir "$Gate-$Profile.txt"
Write-WbEvidence -Path $evidence -Lines $lines -MachineLabel $Machine
Write-Host "${verdict}: $evidence"
if ($verdict -eq 'INVALID') { exit 3 }
if ($verdict -eq 'FAIL') { exit 1 }
