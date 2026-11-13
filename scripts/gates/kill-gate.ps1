<#
.SYNOPSIS
  Phase 1 kill test on the real app (gate build): force-kill mid-recording 5 times, restart,
  resume, then stop and check every part, parts.json and the audio lost at each kill.

.DESCRIPTION
  One session, recorded from the virtual cable while wb-testbench plays a known noise file from
  outside the emulation job. Each round records for 60 to 120 seconds, then `taskkill /F` kills
  the app (no clean shutdown), and:
    1. every part on disk must decode in Media Foundation before any repair (the header flushed
       every second keeps it playable);
    2. the app is started again with WB_GATE_RECOVERY=resume: start-up recovery must find the
       session (log line "offering Resume or Close"), repair it and resume in a new part.
  After the last round the session is stopped. Checks:
    - audio lost at each kill <= 1 s: (kill time - first sample time of that run) - audio recorded
      in that run, from session.json's audio_started events (first sample back-dated from the
      everything queued in the ring when it was first drained; device latency before the first
      callback is on neither side), cross-checked from the source: the forward jump in the
      recorded signal at that restart minus the downtime from the kill to the next run's first
      sample. The larger of the two estimates is gated;
    - kills are aimed at awkward moments: with 30-second parts (WB_GATE_PART_SECONDS), kills 1 and
      3 land about 0.5 s before a part rollover, kill 2 about 0.5 s after one (the new part's
      header is not yet flushed), kills 4 and 5 at random points;
    - every part decodes; parts.json equals a rebuild from the files;
    - against the source, the only discontinuities are the restarts (one per kill, all forward),
      with no silent gaps and no foreign sound;
    - zero overruns in every run; mirror SHA-256 equal at Stop.

  Rules as recorder-gate.ps1: RTSS and MSI Afterburner closed, D:\Projects\stelmar-gate.lock held,
  evidence redacted, audio kept outside the repository.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File .\scripts\gates\kill-gate.ps1
#>
[CmdletBinding()]
param(
  [ValidateSet('emu-low', 'emu-mid', 'native')] [string]$Profile = 'emu-low',
  [int]$Kills = 5,
  [int]$MinRunSeconds = 60,
  [int]$MaxRunSeconds = 120,
  [int]$PartSeconds = 30,
  # Run even though the cable is a default playback device (see recorder-gate.ps1).
  [switch]$AllowCableDefault,
  [string]$Machine = 'Legion',
  [string]$WorkRoot = 'D:\STELMAR-Gate',
  [string]$BackupRoot = 'C:\STELMAR-Gate-Backup',
  # Empty = found automatically and proven with a known signal (scripts\lib\CableRig.psm1).
  [string]$CaptureDevice = '',
  [string]$PlayDevice = '',
  [string]$OutDir,
  [string]$Exe,
  [string]$Testbench,
  [string]$Emulate
)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo = Resolve-Path (Join-Path $here '..\..')
if (-not $OutDir) { $OutDir = Join-Path $repo 'docs\evidence\phase-1' }
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
Import-Module (Join-Path $repo 'scripts\lib\Detector.psm1') -Force
foreach ($f in @($Exe, $Testbench, $Emulate)) { if (-not (Test-Path $f)) { throw "Missing $f; build it first (scripts\gates\build-gate.ps1)" } }

function Q([string]$v) { ConvertTo-WbArg $v }
function Read-Json([string]$Path) { if (Test-Path -LiteralPath $Path) { Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json } else { $null } }
function Get-CableIntruders([string[]]$Names, [int[]]$Allow = @()) {
  @(Get-WbCableIntruders -Testbench $Testbench -Names $Names -Allow $Allow)
}

$logFile = Join-Path $env:LOCALAPPDATA 'in.edu.bmu.stelmar.voiceworkbench\logs\app.log'

$result = $null
try {
$result = Invoke-WbGate -Name "phase1-kill-$Profile" -Body {
  if (Get-Process stelmar-voice-workbench -ErrorAction SilentlyContinue) { throw 'The app is already running; close it first.' }
  # The rig, before anything starts (see recorder-gate.ps1): same cable instance, cable not the
  # default playback device, nobody else playing into it, a known signal heard on the capture.
  Assert-WbNoResidentAudio
  $cableRig = Resolve-WbCableRig -Testbench $Testbench -PlayDevice $PlayDevice -CaptureDevice $CaptureDevice
  $cableRoles = @(Assert-WbCableNotDefault -Testbench $Testbench -Rig $cableRig -Allow:$AllowCableDefault)
  $intruders = @(Get-CableIntruders -Names $cableRig.RenderNames)
  if ($intruders.Count -gt 0) { throw ("RIG REFUSED: Another program is playing into the cable ($($cableRig.RenderNames -join '; ')): " + ($intruders -join ', ')) }
  $cableTest = Invoke-WbRigSelfTest -Rig $cableRig -Testbench $Testbench -WorkRoot $WorkRoot
  $playName = $cableTest.PlayName
  $capName = $cableTest.CaptureName
  $rigPlan = Get-WbRigPlan -Emulate $Emulate -Profile $Profile
  if ($rigPlan) { Assert-WbRigPlacement -JobMask $rigPlan.JobMask -PlayerMask $rigPlan.PlayerMask -ReferenceMask $rigPlan.ReferenceMask -What 'planned, before anything starts' }
  $stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
  $run = Join-Path $WorkRoot "kill-$Profile-$stamp"
  $rec = Join-Path $run 'rec'
  New-Item -ItemType Directory -Force -Path $rec | Out-Null
  $srcDir = Join-Path $WorkRoot 'source'
  New-Item -ItemType Directory -Force -Path $srcDir | Out-Null
  $seconds = ($MaxRunSeconds + 30) * ($Kills + 1) + 120
  $source = Join-Path $srcDir "noise-${seconds}s.wav"
  if (-not (Test-Path ($source + '.json'))) {
    & $Testbench gen --out $source --seconds $seconds --rate 48000 --channels 2 | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'wb-testbench gen failed' }
  }
  $env:WB_GATE_ROOT = $rec
  $env:WB_GATE_BACKUP = Join-Path $BackupRoot "kill-$Profile-$stamp"
  $env:WB_GATE_DEVICE = $capName
  $env:WB_GATE_STATUS_FILE = Join-Path $run 'status.json'
  $env:WB_GATE_REPORT = Join-Path $run 'stop-report.json'
  $env:WB_GATE_STOP_FILE = Join-Path $run 'STOP'
  $env:WB_GATE_PANIC_AT_S = ''
  $env:WB_GATE_RECORDER_PANIC_AT_S = ''
  $env:WB_GATE_PART_SECONDS = "$PartSeconds"
  $logStart = $(if (Test-Path $logFile) { (Get-Item $logFile).Length } else { 0 })
  $progress = New-Object System.Collections.Generic.List[string]
  $interference = New-Object System.Collections.Generic.List[string]
  $rounds = New-Object System.Collections.Generic.List[object]
  $interferenceKeys = @{}
  $cableSeen = @{}
  $pollProblems = New-Object System.Collections.Generic.List[string]

  $refDir = Join-Path $run 'reference'
  $refStop = Join-Path $run 'STOP-REF'
  $goFile = Join-Path $run 'GO-RIG'
  # Their error text goes to files: a player or capture that cannot start must leave its reason.
  $refProc = $null; $play = $null; $current = $null
  $finished = $false
  $playStop = Join-Path $run 'STOP-PLAY'
  # Whatever happens below, nothing started here may outlive the body (see the finally block).
  try {
  $refProc = Start-Process -FilePath $Testbench -ArgumentList @('capture', '--device', (Q $capName), '--dir', (Q $refDir), '--stop-file', (Q $refStop), '--go-file', (Q $goFile), '--report', (Q (Join-Path $run 'reference.json'))) -PassThru -WindowStyle Hidden -RedirectStandardOutput (Join-Path $run 'reference.out') -RedirectStandardError (Join-Path $run 'reference.err')
  Start-Sleep -Seconds 1
  $playStop = Join-Path $run 'STOP-PLAY'
  $play = Start-Process -FilePath $Testbench -ArgumentList @('play', '--device', (Q $playName), '--file', (Q $source), '--stop-file', (Q $playStop), '--go-file', (Q $goFile), '--report', (Q (Join-Path $run 'play.json'))) -PassThru -WindowStyle Hidden -RedirectStandardOutput (Join-Path $run 'play.out') -RedirectStandardError (Join-Path $run 'play.err')
  # Pin the player and the reference capture outside the job's cores at HIGH priority class.
  $rig = Set-WbRigPins -Plan $rigPlan -Player $play -Reference $refProc -GoFile $goFile
  Start-Sleep -Seconds 2

  function Start-App([int]$Index) {
    Remove-Item -LiteralPath $env:WB_GATE_STATUS_FILE -ErrorAction SilentlyContinue
    $inner = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', (Q (Join-Path $here 'recorder-gate-inner.ps1')), '-Exe', (Q $Exe), '-ResultFile', (Q (Join-Path $run "inner-$Index.json")))
    $emuArgs = @('--profile', $Profile, '--log', (Q (Join-Path $run 'emulate.log')), '--', 'powershell.exe') + $inner
    $emu = Start-Process -FilePath $Emulate -ArgumentList $emuArgs -PassThru -WindowStyle Hidden
    $deadline = (Get-Date).AddSeconds(180)
    while ($true) {
      $st = Read-Json $env:WB_GATE_STATUS_FILE
      if ($st -and $st.recorder -and $st.recorder.state -eq 'recording' -and $st.recorder.part) { return [pscustomobject]@{ Emu = $emu; Status = $st } }
      if ($emu.HasExited) { throw "Run $Index ended before recording (exit $($emu.ExitCode)); see $logFile" }
      if ((Get-Date) -gt $deadline) { throw "Run $Index did not start recording within 180 s" }
      Start-Sleep -Milliseconds 250
    }
  }

  # Run 0: a new session.
  $env:WB_GATE_START = "Gate kill $stamp"
  $env:WB_GATE_RECOVERY = ''
  $current = Start-App 0
  $rig = Confirm-WbRigAgainstJob -Rig $rig -Player $play -Reference $refProc
  if ($rig) { $progress.Add("$(Get-Date -Format 'HH:mm:ss') rig pinned: job $(Format-WbMask $rig.Job.Mask), player $(Format-WbMask $rig.Player.Mask), reference capture $(Format-WbMask $rig.Reference.Mask), no overlap") }
  $sessionDir = $current.Status.recorder.session_dir
  $progress.Add("$(Get-Date -Format 'HH:mm:ss') run 0 recording in $sessionDir")
  $env:WB_GATE_START = ''

  $rng = New-Object System.Random
  for ($k = 1; $k -le $Kills; $k++) {
    $wait = $rng.Next($MinRunSeconds, $MaxRunSeconds + 1)
    $until = (Get-Date).AddSeconds($wait)
    while ((Get-Date) -lt $until) {
      # Every session on the cable is recorded by program name and path; a failed look is a note, never an error.
      $poll = Get-WbCablePoll -Testbench $Testbench -Names $cableRig.RenderNames -Allow @($play.Id) -Seen $cableSeen
      foreach ($i in $poll.Intruders) { if (-not $interferenceKeys.ContainsKey($i)) { $interferenceKeys[$i] = $true; $interference.Add("$(Get-Date -Format 'HH:mm:ss') first seen playing: $i") } }
      if ($poll.Error -and $pollProblems.Count -lt 5) { $pollProblems.Add("$(Get-Date -Format 'HH:mm:ss') $($poll.Error)") }
      Start-Sleep -Seconds 1
    }
    # Aim at a part rollover: -500 ms (end of a part) for kills 1 and 3, +500 ms (first second of
    # the next part) for kill 2; kills 4 and 5 stay where the random wait left them.
    $aim = @{ 1 = -500; 2 = 500; 3 = -500 }[$k]
    if ($null -ne $aim) {
      $st = Read-Json $env:WB_GATE_STATUS_FILE
      $age = ([DateTimeOffset]::Now - [DateTimeOffset]::Parse($st.written_at)).TotalMilliseconds
      $toRollover = $PartSeconds * 1000 - [double]$st.recorder.part_audio_ms - $age + $aim
      while ($toRollover -lt 1000) { $toRollover += $PartSeconds * 1000 }
      Start-Sleep -Milliseconds ([int]$toRollover)
    }
    $st = Read-Json $env:WB_GATE_STATUS_FILE
    $app = @(Get-Process stelmar-voice-workbench -ErrorAction SilentlyContinue)
    if ($app.Count -ne 1) { throw "Expected one app process before kill $k, found $($app.Count)" }
    $killAt = [DateTimeOffset]::Now
    $tkRes = Invoke-WbNative -FilePath 'taskkill.exe' -ArgumentList @('/F', '/PID', "$($app[0].Id)")
    $tk = @($tkRes.Output) + @($tkRes.Error)
    $progress.Add("$($killAt.ToString('HH:mm:ss.fff')) kill $k after $wait s: taskkill /F /PID $($app[0].Id): $($tk -join ' ') (last status: part $($st.recorder.part), overrun samples $($st.recorder.overrun_samples))")
    if (-not $current.Emu.WaitForExit(60000)) { throw "Job did not end after kill $k" }
    Start-Sleep -Milliseconds 500
    # Every part must play before any repair.
    $decodeJson = & $Testbench decode --session $sessionDir
    $decode = $decodeJson | Out-String | ConvertFrom-Json
    $rounds.Add([pscustomobject]@{
      Kill = $k; KillAt = $killAt; RanSeconds = $wait; OverrunsBefore = [int64]$st.recorder.overrun_samples + [int64]$st.recorder.dropped_samples
      Aim = $(if ($null -ne $aim) { "$aim ms from a rollover" } else { 'random' })
      PlayableBeforeRepair = [bool]$decode.all_playable
      Parts = (@($decode.parts) | ForEach-Object { "$($_.file) header $($_.header_ms) ms decoded $($_.decoded_ms) ms" }) -join '; '
    })
    # Restart: recovery must offer Resume; the gate hook resumes.
    $env:WB_GATE_RECOVERY = 'resume'
    $current = Start-App $k
    $progress.Add("$(Get-Date -Format 'HH:mm:ss') run $k resumed: part $($current.Status.recorder.part), audio $([math]::Round($current.Status.recorder.audio_ms / 1000.0, 1)) s")
  }
  Start-Sleep -Seconds 30
  $env:WB_GATE_RECOVERY = ''
  Set-Content -LiteralPath $env:WB_GATE_STOP_FILE -Value 'stop'
  if (-not $current.Emu.WaitForExit(600000)) { throw 'The app did not stop' }
  # Ask the player to stop (so it writes its report, with the late-callback count); kill it only if it does not.
  Set-Content -LiteralPath $playStop -Value 'stop'
  if (-not $play.WaitForExit(30000)) { Stop-Process -Id $play.Id -Force -ErrorAction SilentlyContinue }
  Set-Content -LiteralPath $refStop -Value 'stop'
  [void]$refProc.WaitForExit(120000)
  $refAnalysisErr = Invoke-WbAnalyse -Testbench $Testbench -Session $refDir -Source $source -Report (Join-Path $run 'reference-analysis.json')

  $report = Read-Json $env:WB_GATE_REPORT
  $analysisErr = Invoke-WbAnalyse -Testbench $Testbench -Session $sessionDir -Source $source -Report (Join-Path $run 'analysis.json')
  & $Testbench decode --session $sessionDir --report (Join-Path $run 'decode.json') | Out-Null
  & $Testbench parts-check --session $sessionDir --report (Join-Path $run 'parts-check.json') | Out-Null
  $cmp = Invoke-WbCompare -Testbench $Testbench -SessionDir $sessionDir -ReferenceDir $refDir -RunDir $run -IgnoreReferenceTail
  $appLog = @()
  if (Test-Path $logFile) {
    $fs = [System.IO.File]::Open($logFile, 'Open', 'Read', 'ReadWrite')
    try { [void]$fs.Seek($logStart, 'Begin'); $sr = New-Object System.IO.StreamReader($fs); $appLog = $sr.ReadToEnd() -split "`r?`n" } finally { $fs.Dispose() }
  }
  [pscustomobject]@{
    Run = $run; SessionDir = $sessionDir; Report = $report; Rounds = @(ConvertTo-WbArray $rounds); Progress = $progress; Interference = @(ConvertTo-WbArray $interference)
    Analysis = (Read-Json (Join-Path $run 'analysis.json')); Decode = (Read-Json (Join-Path $run 'decode.json'))
    RefAnalysis = (Read-Json (Join-Path $run 'reference-analysis.json')); Reference = (Read-Json (Join-Path $run 'reference.json'))
    AnalysisError = $analysisErr; RefAnalysisError = $refAnalysisErr; CableDefaultRoles = $cableRoles; Rig = $rig; Compare = (Read-Json (Join-Path $run 'compare.json')); CompareError = $(if ($cmp.ExitCode -ne 0) { (@($cmp.Error) + @($cmp.Output)) -join ' ' } else { $null }); Play = (Read-Json (Join-Path $run 'play.json'))
    PartsCheck = (Read-Json (Join-Path $run 'parts-check.json')); Meta = (Read-Json (Join-Path $sessionDir 'session.json')); AppLog = $appLog
    CableRig = $cableRig; CableTest = $cableTest; CableSeen = $cableSeen; CablePollProblems = @(ConvertTo-WbArray $pollProblems)
  }
  $finished = $true
  } finally {
    # On any error in the body: stop the app (its job), the player and the reference capture, so a
    # crash leaves no orphan holding the runner's pipes. On the normal path they have already stopped.
    if (-not $finished) {
      $emus = @(); if ($current) { $emus = @($current.Emu) }
      $left = @(Stop-WbRigLeftovers -Process (@($emus) + @($play, $refProc)) -StopFile @($env:WB_GATE_STOP_FILE, $playStop, $refStop) -GraceSeconds 45)
      foreach ($l in $left) { Write-Host "cleanup after an error: $l" }
    }
  }
}

} catch {
  $detail = $(if ($script:Current -and $script:Current.Error) { $script:Current.Error } else { ($_.Exception.GetType().FullName + ': ' + $_.Exception.Message + "`r`n" + $_.ScriptStackTrace) })
  Write-WbGateCrashEvidence -Path (Join-Path $OutDir "kill-$Profile.txt") -Reason "the kill gate stopped on an error: $($_.Exception.Message)" -Detail $detail -Machine $Machine -Profile $Profile -Builds @($Exe, $Testbench, $Emulate)
  Write-Host "FAIL: the kill gate did not complete: $($_.Exception.Message)"
  exit 1
}

# ---- Loss per kill, verdict and evidence ----
$r = @($result)[-1]
$missing = @(Test-WbGateData -Result $r -Required @('Report', 'Analysis', 'Decode', 'RefAnalysis', 'Reference', 'PartsCheck', 'Meta'))
if ($missing.Count -eq 0 -and -not ((Test-WbHasProperty $r.Report 'stopped') -and $null -ne $r.Report.stopped)) { $missing = @('Report.stopped (the app did not report a stop)') }
if ($missing.Count -gt 0) {
  $extra = @("missing: $($missing -join ', ')")
  if ($r.AnalysisError) { $extra += "analyser (app recording): $($r.AnalysisError)" }
  if ($r.RefAnalysisError) { $extra += "analyser (reference capture): $($r.RefAnalysisError)" }
  $extra += "run folder: $($r.Run) (kept; nothing was deleted)"
  Write-WbGateCrashEvidence -Path (Join-Path $OutDir "kill-$Profile.txt") -Reason 'no verdict data: the run produced no analysis or reports to judge' -Machine $Machine -Profile $Profile -Builds @($Exe, $Testbench, $Emulate) -Extra $extra
  Write-Host "FAIL: no verdict data ($($missing -join ', '))"
  exit 1
}
$events = @($r.Meta.events)
# Breaks also in the independent reference capture are the source chain's, not restarts.
$split = Split-WbDiscontinuities -App $r.Analysis -Reference $r.RefAnalysis
$disc = @($split.Own)
$started = @($events | Where-Object { $_.kind -eq 'audio_started' })
$recovered = @($events | Where-Object { $_.kind -eq 'recovered' })
$lossLines = @()
$maxLoss = 0.0
for ($i = 0; $i -lt $r.Rounds.Count; $i++) {
  $round = $r.Rounds[$i]
  if ($i -ge $started.Count -or $i -ge $recovered.Count) { $lossLines += "  kill $($round.Kill): missing audio_started or recovered event"; $maxLoss = [double]::PositiveInfinity; continue }
  $s0 = [DateTimeOffset]::Parse($started[$i].at)
  $a0 = [double]$started[$i].audio_ms
  $a1 = [double]$recovered[$i].audio_ms
  $wallMs = ($round.KillAt - $s0).TotalMilliseconds
  $recMs = $a1 - $a0
  $loss = $wallMs - $recMs
  # Cross-check from the source: the jump in the recorded signal at this restart is the audio
  # played between the last sample kept and the first sample of the next run.
  $loss2 = [double]::NaN
  if ($i -lt $disc.Count -and ($i + 1) -lt $started.Count) {
    $downMs = ([DateTimeOffset]::Parse($started[$i + 1].at) - $round.KillAt).TotalMilliseconds
    $loss2 = [double]$disc[$i].source_jump_ms - $downMs
  }
  $worst = $(if ([double]::IsNaN($loss2)) { [double]::PositiveInfinity } else { [math]::Max($loss, $loss2) })
  if ($worst -gt $maxLoss) { $maxLoss = $worst }
  $landed = $recMs % ($PartSeconds * 1000)
  $lossLines += ("  kill {0} ({1}; landed {2:n0} ms into a part): first sample {3}, killed {4}: {5:n0} ms of wall time, {6:n0} ms recorded, lost {7:n0} ms; from the source signal {8:n0} ms; parts playable before repair: {9}" -f $round.Kill, $round.Aim, $landed, $s0.ToString('HH:mm:ss.fff'), $round.KillAt.ToString('HH:mm:ss.fff'), $wallMs, $recMs, $loss, $loss2, $round.PlayableBeforeRepair)
}
$offered = @($r.AppLog | Where-Object { $_ -match 'offering Resume or Close' }).Count
$stopped = $r.Report.stopped
$checks = [ordered]@{}
$checks["audio lost at each kill <= 1000 ms (max $([math]::Round($maxLoss)) ms)"] = ($maxLoss -le 1000)
$checks['every part played before repair, after each kill'] = -not (@($r.Rounds | Where-Object { -not $_.PlayableBeforeRepair }).Count -gt 0)
$checks['every part decodes in Media Foundation at the end'] = [bool]$r.Decode.all_playable
$checks["restart offered Resume after every kill ($offered of $($r.Rounds.Count))"] = ($offered -ge $r.Rounds.Count)
$checks['parts.json equals a rebuild from the part files'] = [bool]$r.PartsCheck.equal
$checks["only the restarts break the audio ($($disc.Count) forward jumps for $($r.Rounds.Count) kills)"] = ($disc.Count -eq $r.Rounds.Count -and -not ($disc | Where-Object { [int64]$_.source_jump_frames -le 0 }))
$refUnmatched = $(if ($r.RefAnalysis) { [double]$r.RefAnalysis.unmatched_ms } else { 0 })
# The rule of 9 Nov 2026 (docs/PLAN.md section 12): each run of the app is judged against the
# reference capture, with one fixed offset per run; breaks between the source and the reference
# are reported and judged for validity, not failed.
$killPlayStart = $(if ($r.Play -and (Test-WbHasProperty $r.Play 'started_at')) { [DateTimeOffset]::Parse($r.Play.started_at) } else { $null })
$rv = Get-WbRunVerdict -Compare $r.Compare -RefAnalysis $r.RefAnalysis -Play $r.Play -Reference $r.Reference -PlayStarted $killPlayStart -DroppedSamples ([int64]$stopped.recorder.dropped_samples)
$fidelityText = $(if ($r.Compare) { "$(@($r.Compare.segments).Count) runs, differing frames $($r.Compare.differing_frames), reference audio without an app recording $($r.Compare.reference_only_loud_frames) frames" } else { 'no comparison data' })
$checks["fidelity: each run of the app equals the reference capture sample for sample, one fixed offset per run ($fidelityText)"] = ($null -ne $r.Compare -and [bool]$r.Compare.aligned -and [bool]$r.Compare.equal)
$checks['no other program played into the cable'] = ($r.Interference.Count -eq 0)
$checks['reference capture ran cleanly'] = ($r.Reference -and [int64]$r.Reference.overrun_samples -eq 0 -and $r.RefAnalysis)
$checks['zero ring-buffer overruns and dropped samples (final run, and before every kill)'] = ([int64]$stopped.recorder.overrun_samples + [int64]$stopped.recorder.dropped_samples -eq 0 -and -not ($r.Rounds | Where-Object { $_.OverrunsBefore -ne 0 }))
$lossFree = ($null -ne $r.Compare -and [bool]$r.Compare.equal)
$xr = Get-WbXrunVerdict -SessionDir $r.SessionDir -LossFree $lossFree
$checks["driver xruns: at most one per run, within 2 s of the stream start, zero loss or duplication around it (total $($xr.TotalXruns), excused $(@($xr.Excused).Count))"] = $xr.Pass
$checks['mirror SHA-256 equals the originals at Stop'] = [bool]$stopped.mirror.all_match
$pass = -not ($checks.Values -contains $false) -and $rv.Verdict -ne 'FAIL'
$verdict = $(if ($rv.Verdict -eq 'INVALID') { 'INVALID' } elseif ($pass) { 'PASS' } else { 'FAIL' })

$lines = @()
$lines += Get-WbGateHeader -Machine $Machine -Profile $Profile -Builds @($Exe, $Testbench, $Emulate)
$lines += "parts: $PartSeconds s for this test (WB_GATE_PART_SECONDS), so kills can be aimed at rollovers; not covered here: kills during Pause or while the input is lost"
$lines += 'app build: target\gate\stelmar-voice-workbench.exe (cargo profile gate, features gate-hooks + custom-protocol)'
$lines += "verdict: $(if ($verdict -eq 'INVALID') { 'INVALID (no verdict; the run must be repeated)' } elseif ($verdict -eq 'PASS' -and $Profile -ne 'native') { 'PASS (PROVISIONAL: emulated)' } else { $verdict })"
$lines += ''
$lines += 'checks:'
foreach ($k in $checks.Keys) { $lines += "  [$(if ($checks[$k]) { 'x' } else { ' ' })] $k" }
$lines += ''
$lines += Get-WbVerdictLines -Result $rv -Compare $r.Compare
if ($r.CompareError) { $lines += "comparison error: $($r.CompareError)" }
$lines += ''
$lines += 'loss per kill (method: kill time minus that run''s first-sample time, minus the audio that run left on disk after repair):'
$lines += $lossLines
$lines += ''
$lines += 'parts after each kill, before repair (Media Foundation decode):'
$lines += ($r.Rounds | ForEach-Object { "  kill $($_.Kill): $($_.Parts)" })
$lines += ''
$lines += ("final: {0} parts, {1:n1} s of audio, mirror {2} files equal {3} (mirror on the same machine, separate drive pending)" -f @($stopped.recorder.parts).Count, ($stopped.recorder.audio_ms / 1000.0), @($stopped.mirror.files).Count, $stopped.mirror.all_match)
$lines += ("driver xruns (rule: one per run within 2 s of the stream start, reported not gated, only with zero loss and duplication): total {0}; excused at audio-time offsets {1} ms; problems: {2}" -f $xr.TotalXruns, ((@($xr.Excused) | ForEach-Object { [math]::Round($_.OffsetMs) }) -join ', '), $(if (@($xr.Problems).Count) { @($xr.Problems) -join '; ' } else { 'none' }))
$lines += "parts.json check: $($r.PartsCheck | ConvertTo-Json -Compress)"
$lines += Get-WbCableRigLines -Rig $r.CableRig -SelfTest $r.CableTest
$lines += Get-WbRigLines -Rig $r.Rig -Play $r.Play -Reference $r.Reference
# Live detector check (Phase 2): shared upstream breaks against the repeated_block and zero_gap
# events in session.json (all runs of the app, a kill restarts the recorder). Not gated.
$sessionJson = Join-Path $r.SessionDir 'session.json'
if ($r.Analysis -and (Test-Path -LiteralPath $sessionJson)) {
  $sessionMeta = Get-Content -LiteralPath $sessionJson -Raw | ConvertFrom-Json
  $detCheck = Get-WbDetectorMatches -Breaks @($rv.Breaks) -Analysis $r.Analysis -Events @($sessionMeta.events)
  $lines += Get-WbDetectorLines -Check $detCheck -Rec $null
  Save-WbDetectorCheck -RunDir $r.Run -Gate 'kill' -Profile $Profile -Check $detCheck -Rec $stopped.recorder -AudioMs ([double]$stopped.recorder.audio_ms) -Verdict $verdict
  $lines += '  (position counters are per run of the app; the force-killed runs have no report, so none are given here)'
} else {
  $lines += 'live detector check: not available (no analysis or session.json for this run)'
}
$killSplit = Split-WbBreakSources -App $r.Analysis -Reference $r.RefAnalysis
$lines += Get-WbLateCallbackLines -Play $r.Play -BreakSourceMs @($killSplit.Shared) -Label 'shared break'
$lines += Get-WbLateCallbackLines -Play $r.Play -BreakSourceMs @($killSplit.AppOnly) -Label 'app-only break (not in the reference capture)' | Where-Object { $_ -match 'app-only break' }
$lines += "cable interference: $(if ($r.Interference.Count) { $r.Interference -join '; ' } else { 'none' })"
if ($null -ne $r.CableSeen) { $lines += @(Get-WbCableSeenLines -Seen $r.CableSeen -PollError (@($r.CablePollProblems) -join '; ')) }
$lines += $(if (@($r.CableDefaultRoles).Count -gt 0) { "default playback device: THE CABLE for $(@($r.CableDefaultRoles) -join ', ') (run with -AllowCableDefault: other sounds on this computer can reach the test signal)" } else { 'default playback device: not the cable (checked at start)' })
$killPlayStart = $(if ($r.Play -and (Test-WbHasProperty $r.Play 'started_at')) { [DateTimeOffset]::Parse($r.Play.started_at) } else { $null })
$lines += Get-WbReferenceXrunLines -RefAnalysis $r.RefAnalysis -Reference $r.Reference -PlayStarted $killPlayStart -BreakSourceMs @(@($r.Analysis.unmatched_ranges) | ForEach-Object { [double]$_.source_ms })
$lines += "reference capture: $(@($r.RefAnalysis.discontinuities).Count) breaks; in the app recording: restarts and own $($split.Own.Count), shared with the reference (upstream, see the upstream breaks section) $($split.Chain.Count), start-up repeats within the old allowance $($split.Excused.Count)"
$lines += ''
$lines += 'discontinuities against the source (each should be one restart):'
$lines += ($disc | ForEach-Object { "  $($_ | ConvertTo-Json -Compress)" })
$lines += ''
$lines += 'progress:'
$lines += ($r.Progress | ForEach-Object { "  $_" })
$lines += ''
$lines += 'session events:'
$lines += ($events | ForEach-Object { "  $($_.at) audio_ms=$($_.audio_ms) $($_.kind) $($_.detail)" })
$lines += ''
$lines += 'app log for this run (recovery, session, warnings):'
$lines += (@($r.AppLog) | Where-Object { $_ -match 'recovery|offering|gate|session|WARN|ERROR' } | Select-Object -Last 80 | ForEach-Object { "  $_" })
$lines += ''
$lines += "audio and reports: $($r.Run) (not in the repository)"
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$evidence = Join-Path $OutDir "kill-$Profile.txt"
Write-WbEvidence -Path $evidence -Lines $lines -MachineLabel $Machine
Write-Host "${verdict}: $evidence"
if ($verdict -eq 'INVALID') { exit 3 }
if ($verdict -eq 'FAIL') { exit 1 }
