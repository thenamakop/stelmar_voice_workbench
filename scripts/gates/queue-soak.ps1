<#
.SYNOPSIS
  Queue soak: the recorder beside the app's real transcription queue, two sessions through the cable.

.DESCRIPTION
  Runs the gate build of the app (target\gate, cargo feature gate-hooks) with WB_GATE_DAY_PLAN: session 1 is public
  speech (the FLEURS English and Hindi clips of resources\bench, CC BY 4.0, looped with pauses) that the app's own
  queue transcribes - part by part while it records, then what is left - while session 2, seeded noise, is recorded
  through the same cable. The recorder is judged on both sessions by the Phase 1 recording rule; session 1's
  transcript is judged for completeness. See scripts\lib\QueueSoak.psm1 for the rules.

  The app gets a data folder of its own (WB_GATE_DATA_DIR, as every gate); the capability.json of THIS pack run's quick
  check is copied into it (-CapabilityFile): without a system check the queue creates no job. The gate pack's wrapper
  (scripts\gate-pack\gates\queue-soak.ps1) answers PENDING when there is no such file and never starts this script.

  Exit code: 0 PASS, 1 FAIL, 3 INVALID, 4 NOT APPLICABLE (this machine's system check did not clear processing while
  recording, so the queue correctly waits and there is nothing to measure).

  Needs: a rig (a hardware loopback with -RigPrefer loopback, or VB-CABLE), the gate build, wb-testbench, the engines and
  models next to the build, and wb-emulate for an emulation profile (not for -Profile laptop).

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File .\scripts\gates\queue-soak.ps1 -Profile emu-low -CapabilityFile D:\...\capability.json -Quick
#>
[CmdletBinding()]
param(
  [ValidateSet('emu-low', 'emu-mid', 'native', 'laptop')] [string]$Profile = 'emu-low',
  [ValidateSet('', 'loopback', 'cable')] [string]$RigPrefer = '',
  [Parameter(Mandatory = $true)] [string]$CapabilityFile,
  # The folder with bench-en.wav and bench-hi.wav (public FLEURS, CC BY 4.0): the speech of session 1.
  [string]$BenchClips,
  [switch]$Quick,
  [int]$Session1Seconds = 0,
  [int]$Session2Seconds = 0,
  [int]$PartSeconds = -1,
  [string]$Machine = 'Legion',
  [string]$WorkRoot = 'D:\STELMAR-Gate',
  [string]$BackupRoot = 'C:\STELMAR-Gate-Backup',
  [string]$CaptureDevice = '',
  [string]$PlayDevice = '',
  [string]$OutDir,
  [string]$Exe,
  [string]$Testbench,
  [string]$Emulate,
  [switch]$AllowCableDefault
)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo = (Resolve-Path (Join-Path $here '..\..')).Path
if (-not $OutDir) { $OutDir = Join-Path $repo 'docs\evidence\gate-pack-dryrun' }
if (-not $Exe) { $Exe = Join-Path $repo 'target\gate\stelmar-voice-workbench.exe' }
if (-not $Testbench) { $Testbench = Join-Path $repo 'target\release\wb-testbench.exe' }
if (-not $BenchClips) { $BenchClips = Join-Path $repo 'resources\bench' }
$isLaptop = ($Profile -eq 'laptop')
if (-not $Emulate -and -not $isLaptop -and $Profile -ne 'native') { $Emulate = Join-Path $repo 'target\release\wb-emulate.exe' }
Import-Module (Join-Path $repo 'scripts\lib\PsEnv.psm1') -Force
Set-WbCleanPSModulePath
Set-WbInvariantCulture
Import-Module (Join-Path $repo 'scripts\lib\GateRun.psm1') -Force
Import-Module (Join-Path $repo 'scripts\lib\GateData.psm1') -Force
Import-Module (Join-Path $repo 'scripts\lib\Redact.psm1') -Force
Import-Module (Join-Path $repo 'scripts\lib\AppProcess.psm1') -Force
Import-Module (Join-Path $repo 'scripts\lib\Rig.psm1') -Force
Import-Module (Join-Path $repo 'scripts\lib\CableRig.psm1') -Force
Import-Module (Join-Path $repo 'scripts\lib\Fidelity.psm1') -Force
Import-Module (Join-Path $repo 'scripts\lib\RefVerdict.psm1') -Force
Import-Module (Join-Path $repo 'scripts\lib\DaySim.psm1') -Force
Import-Module (Join-Path $repo 'scripts\lib\QueueSoak.psm1') -Force
if ($RigPrefer) { Import-Module (Join-Path $repo 'scripts\lib\LoopbackRig.psm1') -Force }
$builds = @($Exe, $Testbench)
if ($Emulate) { $builds += $Emulate }
foreach ($f in $builds) { if (-not (Test-Path -LiteralPath $f)) { throw "Missing $f; build it first (scripts\gates\build-gate.ps1)" } }
foreach ($c in 'bench-en.wav', 'bench-hi.wav') { if (-not (Test-Path -LiteralPath (Join-Path $BenchClips $c))) { throw "Missing $(Join-Path $BenchClips $c) (the public FLEURS clips of the speech session)" } }
$settings = Get-WbQueueSoakSettings -Quick:$Quick
if ($Session1Seconds -le 0) { $Session1Seconds = $settings.Session1Seconds }
if ($Session2Seconds -le 0) { $Session2Seconds = $settings.Session2Seconds }
if ($PartSeconds -lt 0) { $PartSeconds = $settings.PartSeconds }
$gateName = 'queue-soak'
$titles = Get-WbQueueSoakTitles

function Q([string]$v) { ConvertTo-WbArg $v }
function Read-Json([string]$Path) { if (Test-Path -LiteralPath $Path) { Get-Content -LiteralPath $Path -Raw -Encoding UTF8 | ConvertFrom-Json } else { $null } }
function Write-Utf8([string]$Path, [string]$Text) { [IO.File]::WriteAllText($Path, $Text, (New-Object Text.UTF8Encoding($false))) }
function ConvertTo-Slug([string]$Title) { (($Title.ToLowerInvariant() -replace '[^a-z0-9]+', '-').Trim('-')) }

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

# One session's recording against the references. The speech session has no analyser (wb-testbench analyse knows only
# its seeded noise): the app is compared with each reference sample for sample and nothing else about the source is
# claimed. The noise session gets the whole Phase 1 analysis.
function Get-SessionJudging {
  param([int]$Index, [string]$Title, [string]$Source, [bool]$Noise, $SessionReport, [string]$Run)
  $fid = Join-Path $Run "fid-$Index"
  New-Item -ItemType Directory -Force -Path $fid | Out-Null
  $sessionDir = [string]$SessionReport.dir
  $refDir = Join-Path $Run "reference-$Index"
  $ref2Dir = Join-Path $Run "reference2-$Index"
  $play = Read-Json (Join-Path $Run "play-$Index.json")
  $reference = Read-Json (Join-Path $Run "reference-$Index.json")
  $reference2 = Read-Json (Join-Path $Run "reference2-$Index.json")
  $notes = New-Object System.Collections.Generic.List[string]
  $analysis = $null; $refAnalysis = $null
  if ($Noise) {
    $e1 = Invoke-WbAnalyse -Testbench $Testbench -Session $sessionDir -Source $Source -Report (Join-Path $fid 'analysis.json')
    $e2 = Invoke-WbAnalyse -Testbench $Testbench -Session $refDir -Source $Source -Report (Join-Path $fid 'reference-analysis.json')
    $e3 = Invoke-WbAnalyse -Testbench $Testbench -Session $ref2Dir -Source $Source -Report (Join-Path $fid 'reference2-analysis.json')
    foreach ($pair in @(@($e1, 'app'), @($e2, 'cpal reference'), @($e3, 'WASAPI reference'))) { if ($pair[0]) { $notes.Add("analyser ($($pair[1])): $($pair[0])") } }
    $analysis = Read-Json (Join-Path $fid 'analysis.json')
    $refAnalysis = Read-Json (Join-Path $fid 'reference-analysis.json')
  }
  $startMsOf = { param($f) $j = Read-Json (Join-Path $fid $f); if ($j -and (Test-WbHasProperty $j 'content_start_ms')) { ([double]$j.content_start_ms).ToString([Globalization.CultureInfo]::InvariantCulture) } else { $null } }
  $sApp = & $startMsOf 'analysis.json'; $sRef = & $startMsOf 'reference-analysis.json'; $sRef2 = & $startMsOf 'reference2-analysis.json'
  $cmp = Invoke-WbCompare -Testbench $Testbench -SessionDir $sessionDir -ReferenceDir $refDir -RunDir $fid
  if ($cmp.ExitCode -ne 0) { $notes.Add("comparison with the cpal reference: $((@($cmp.Error) + @($cmp.Output)) -join ' ')") }
  $a = @('compare', '--app', $sessionDir, '--reference', $ref2Dir, '--report', (Join-Path $fid 'compare2.json'))
  if ($sApp) { $a += @('--app-signal-start-ms', $sApp) }
  if ($sRef2) { $a += @('--reference-signal-start-ms', $sRef2) }
  $c2 = Invoke-WbNative -FilePath $Testbench -ArgumentList $a
  if ($c2.ExitCode -ne 0) { $notes.Add("comparison with the WASAPI reference: $((@($c2.Error) + @($c2.Output)) -join ' ')") }
  $a = @('compare', '--app', $refDir, '--reference', $ref2Dir, '--report', (Join-Path $fid 'compare-refs.json'))
  if ($sRef) { $a += @('--app-signal-start-ms', $sRef) }
  if ($sRef2) { $a += @('--reference-signal-start-ms', $sRef2) }
  $null = Invoke-WbNative -FilePath $Testbench -ArgumentList $a
  $compare = Read-Json (Join-Path $fid 'compare.json')
  $compare2 = Read-Json (Join-Path $fid 'compare2.json')
  $compareRefs = Read-Json (Join-Path $fid 'compare-refs.json')
  $refV = Get-PackRefVerdict -AppCpal $compare -AppWasapi $compare2 -CpalWasapi $compareRefs
  $rec = $null
  if ($SessionReport.stopped -and $SessionReport.stopped.recorder) { $rec = $SessionReport.stopped.recorder }
  $dropped = [int64]0; $overrun = [int64]0; $xruns = [int64]0
  if ($rec) { $dropped = [int64]$rec.dropped_samples; $overrun = [int64]$rec.overrun_samples; $xruns = [int64]$rec.xruns }
  $lossFree = ($null -ne $compare -and [bool]$compare.equal)
  $xr = $null
  if ($xruns -gt 0) { $xr = Get-WbXrunVerdict -SessionDir $sessionDir -LossFree $lossFree }
  $xrExcused = $(if ($xruns -eq 0) { $true } else { [bool]$xr.Pass })
  $rv = $null
  if ($Noise -and $play -and $reference) {
    try {
      $rv = Get-WbRunVerdict -Compare $compare -RefAnalysis $refAnalysis -Play $play -Reference $reference -PlayStarted ([DateTimeOffset]::Parse($play.started_at)) -DroppedSamples $dropped
    } catch { $notes.Add("the Phase 1 verdict could not be worked out: $($_.Exception.Message)") }
  }
  $refOverrun = [int64]0
  foreach ($rr in @($reference, $reference2)) { if ($rr -and (Test-WbHasProperty $rr 'overrun_samples')) { $refOverrun += [int64]$rr.overrun_samples } }
  $verdict = Get-WbQueueSoakRecordingVerdict -Label "session $Index ($Title)" -RunVerdict $rv -RefVerdict $refV -Dropped $dropped -Overrun $overrun -Xruns $xruns -XrunExcused $xrExcused -ReferenceOverrun $refOverrun
  if ($Noise -and $null -eq $rv) { $verdict = Get-WbQueueSoakRecordingVerdict -Label "session $Index ($Title)" -RunVerdict ([pscustomobject]@{ Verdict = 'INVALID'; InvalidReasons = @('the Phase 1 verdict of the noise session could not be worked out'); FailReasons = @() }) -RefVerdict $refV -Dropped $dropped -Overrun $overrun -Xruns $xruns -XrunExcused $xrExcused -ReferenceOverrun $refOverrun }
  $lines = New-Object System.Collections.Generic.List[string]
  $lines.Add(("session {0} ({1}), {2}: {3}" -f $Index, $Title, $(if ($Noise) { 'seeded noise, full Phase 1 analysis' } else { 'public speech, judged by equality with both references' }), $verdict.Status))
  foreach ($l in @($refV.Lines)) { $lines.Add($l) }
  $lines.Add(('  two references: {0} - {1}' -f $refV.Verdict, $refV.Reason))
  if ($rv) { foreach ($l in @(Get-WbVerdictLines -Result $rv -Compare $compare)) { $lines.Add("  $l") } }
  else { $lines.Add('  upstream breaks of the source: not counted for a speech source (the analyser knows only the seeded noise); the app is judged against what both references captured') }
  if ($rec) { $lines.Add(('  recorder: audio {0:n1} s; overrun samples {1}; driver xruns {2}{3}; samples dropped by the writer {4}; recorder restarts {5}' -f ($rec.audio_ms / 1000.0), $overrun, $xruns, $(if ($xruns -gt 0) { " (excused: $xrExcused)" } else { '' }), $dropped, $rec.recorder_restarts)) }
  if ($SessionReport.stopped -and $SessionReport.stopped.mirror) { $lines.Add(('  mirror: SHA-256 of the copies equals the originals: {0}' -f $SessionReport.stopped.mirror.all_match)) }
  if ($play) { $lines.Add(('  player: underrun callbacks {0}' -f $play.underrun_callbacks)) }
  foreach ($n in $notes) { $lines.Add("  note: $n") }
  [pscustomobject]@{ Verdict = $verdict; Lines = @($lines.ToArray()); AudioMs = $(if ($rec) { [double]$rec.audio_ms } else { 0.0 }) }
}

$result = $null
try {
  $result = Invoke-WbGate -Name "queue-soak-$Profile" -Body {
    if (Get-Process stelmar-voice-workbench -ErrorAction SilentlyContinue) { throw 'The app is already running; close it first.' }
    $capCheck = Get-WbQueueSoakCapability -Path $CapabilityFile
    if (-not $capCheck.Found) { throw "NEEDS A SYSTEM CHECK: $($capCheck.Reason)" }
    # The rig first, as every recorder gate: paired endpoints, the cable not the default device, nobody else playing
    # into it, a known signal heard on the capture.
    Assert-WbNoResidentAudio
    if ($RigPrefer) { $cableRig = Resolve-PackRig -Testbench $Testbench -WorkRoot $WorkRoot -Prefer $RigPrefer -CaptureDevice $CaptureDevice -PlayDevice $PlayDevice }
    else { $cableRig = Resolve-WbCableRig -Testbench $Testbench -PlayDevice $PlayDevice -CaptureDevice $CaptureDevice }
    $null = @(Assert-WbCableNotDefault -Testbench $Testbench -Rig $cableRig -Allow:$AllowCableDefault)
    $intruders = @(Get-WbCableIntruders -Testbench $Testbench -Names $cableRig.RenderNames)
    if ($intruders.Count -gt 0) { throw ("RIG REFUSED: another program is playing into the cable ($($cableRig.RenderNames -join '; ')): " + ($intruders -join ', ')) }
    if ($RigPrefer) { $cableTest = $cableRig.SelfTest } else { $cableTest = Invoke-WbRigSelfTest -Rig $cableRig -Testbench $Testbench -WorkRoot $WorkRoot }
    if ($RigPrefer) { foreach ($b in @($cableRig.Banner)) { Write-Host $b } }
    $playName = $cableTest.PlayName
    $capName = $cableTest.CaptureName
    $rigPlan = $null
    if ($Emulate) { $rigPlan = Get-WbRigPlan -Emulate $Emulate -Profile $Profile }
    if ($rigPlan) { Assert-WbRigPlacement -JobMask $rigPlan.JobMask -PlayerMask $rigPlan.PlayerMask -ReferenceMask $rigPlan.ReferenceMask -What 'planned, before anything starts' }

    $stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
    $run = Join-Path $WorkRoot "$gateName-$Profile-$stamp"
    $rec = Join-Path $run 'rec'
    $backup = Join-Path $BackupRoot "$gateName-$Profile-$stamp"
    New-Item -ItemType Directory -Force -Path $rec | Out-Null
    $srcDir = Join-Path $WorkRoot 'source'
    New-Item -ItemType Directory -Force -Path $srcDir | Out-Null
    # The app's own data folder for this run, with the quick check's capability.json: never the operator's real folder.
    $dataDir = Set-WbGateDataDir -Dir (Join-Path $run 'appdata')
    Copy-Item -LiteralPath $CapabilityFile -Destination (Join-Path $dataDir 'capability.json') -Force
    $startedAt = Get-Date
    Write-Host "$(Get-Date -Format 'HH:mm:ss') run folder $run ($($settings.Mode) mode: session 1 $Session1Seconds s of speech, session 2 $Session2Seconds s of noise)"

    # The two sources. Speech: the public FLEURS English and Hindi clips looped with 2 s pauses (the cycle repeats; the
    # gate judges completeness and order of the transcript, not its words). Noise: seeded, known to the analyser.
    $speechSrc = Join-Path $srcDir ("queue-soak-speech-${Session1Seconds}s.wav")
    $clips = (Join-Path $BenchClips 'bench-en.wav') + ',' + (Join-Path $BenchClips 'bench-hi.wav')
    $mix = Invoke-WbNative -FilePath $Testbench -ArgumentList @('mix', '--out', $speechSrc, '--clips', $clips, '--seconds', "$Session1Seconds", '--pause-s', '2')
    if ($mix.ExitCode -ne 0 -or -not (Test-Path -LiteralPath $speechSrc)) { throw ('wb-testbench mix failed: ' + ((@($mix.Output) + @($mix.Error)) -join ' ')) }
    $noiseSrc = Join-Path $srcDir ("noise-${Session2Seconds}s.wav")
    if (-not (Test-Path -LiteralPath ($noiseSrc + '.json'))) {
      & $Testbench gen --out $noiseSrc --seconds $Session2Seconds --rate 48000 --channels 2 | Out-Null
      if ($LASTEXITCODE -ne 0) { throw 'wb-testbench gen failed' }
    }
    $sources = @($speechSrc, $noiseSrc)
    $slug1 = ConvertTo-Slug $titles.Speech
    $slug2 = ConvertTo-Slug $titles.Noise

    # The plan: session 1 at once, session 2 after it; each session is stopped by its own stop file, written 15 s after
    # its player has finished (DaySim.psm1), never by a timer.
    $plan = New-WbQueueSoakPlan -Session1Seconds $Session1Seconds -Session2Seconds $Session2Seconds -RunDir $run -Device $capName
    $planFile = Join-Path $run 'day-plan.json'
    Write-Utf8 $planFile ($plan | ConvertTo-Json -Depth 6)
    $dayStop = Join-Path $run 'STOP'
    $queueLog = Join-Path $run 'queue.log'
    $reportFile = Join-Path $run 'report.json'
    $tenIst = Get-Date -Hour 10 -Minute 0 -Second 0 -Millisecond 0
    $env:WB_GATE_ROOT = $rec
    $env:WB_GATE_BACKUP = $backup
    $env:WB_GATE_DAY_PLAN = $planFile
    $env:WB_GATE_DAY_STOP_FILE = $dayStop
    $env:WB_GATE_REPORT = $reportFile
    $env:WB_GATE_QUEUE_LOG = $queueLog
    $env:WB_GATE_CLOCK_OFFSET_S = "$([int](($tenIst - (Get-Date)).TotalSeconds))"
    # No night pass while the gate runs: it would wait the real 30 minutes after the last session, and it is not what is measured.
    $env:WB_GATE_NIGHT_WAIT_S = '7200'
    $env:WB_GATE_PART_SECONDS = $(if ($PartSeconds -gt 0) { "$PartSeconds" } else { '' })
    foreach ($v in 'WB_GATE_START', 'WB_GATE_DEVICE', 'WB_GATE_STOP_FILE', 'WB_GATE_STATUS_FILE', 'WB_GATE_FORCE_HYBRID', 'WB_GATE_CHECK') { Remove-Item "Env:\$v" -ErrorAction SilentlyContinue }

    $startApp = {
      param([string]$Mode)
      if ($Emulate) { Start-Process -FilePath $Emulate -ArgumentList @('--profile', $Profile, '--log', (Q (Join-Path $run "emulate-$Mode.log")), '--', (Q $Exe)) -PassThru -WindowStyle Hidden }
      else { Start-Process -FilePath $Exe -PassThru -WindowStyle Hidden }
    }
    # The app's own view of the saved profile: a fingerprint change would decide with safe defaults, and a machine whose
    # check did not clear processing while recording correctly does not transcribe during a recording (nothing to measure).
    $env:WB_GATE_CHECK = 'view'
    $viewReport = $env:WB_GATE_REPORT
    $env:WB_GATE_REPORT = Join-Path $run 'view.json'
    $viewProc = & $startApp 'view'
    if (-not $viewProc.WaitForExit(300000)) { Stop-Process -Id $viewProc.Id -Force; throw 'the profile view did not finish within 5 minutes' }
    Remove-Item Env:\WB_GATE_CHECK -ErrorAction SilentlyContinue
    $env:WB_GATE_REPORT = $viewReport
    $view = Read-Json (Join-Path $run 'view.json')
    if (-not $view -or -not $view.view -or -not $view.view.capability) { throw "the app could not read the saved system check (see $(Join-Path $run 'view.json'))" }
    $changes = @($view.view.fingerprint_changes)
    $decision = $view.view.capability.decision
    Write-Host ("{0} profile: tier {1}, process while recording {2}, fingerprint changes {3}" -f (Get-Date -Format 'HH:mm:ss'), $decision.tier, $decision.process_while_recording, $changes.Count)
    if ($changes.Count -gt 0) { throw "the saved system check no longer matches this build or machine (fingerprint changes: $($changes -join '; ')); run the quick check again" }
    if (-not [bool]$decision.process_while_recording) {
      return [pscustomobject]@{ NotApplicable = $true; Run = $run; Decision = $decision; Settings = $settings; CableRig = $cableRig; CableTest = $cableTest }
    }

    # The run.
    $emu = $null; $players = @(); $refs = @(); $refs2 = @()
    $startedTitles = @{}; $playerExit = @{}; $stopWritten = @{}; $refStopSeen = @{}; $refStopWritten = @{}
    $units1 = New-Object System.Collections.Generic.List[object]
    $progress = New-Object System.Collections.Generic.List[string]
    $lines = 0; $allRecorded = $null; $s1Complete = $null; $s2Started = $null; $s2Stopped = $null; $dayStopAt = $null; $rigConfirmed = $false
    $hardLimit = $startedAt.AddMinutes($settings.MaxMinutes)
    $finished = $false
    try {
      $emu = & $startApp 'day'
      while (-not $emu.HasExited) {
        if ((Get-Date) -gt $hardLimit) { throw "the run did not finish within $($settings.MaxMinutes) minutes" }
        $ref = [ref]$lines
        foreach ($l in (Read-NewLines $queueLog $ref)) {
          if ($l -match 'DAY: started "(.+?)" in') {
            $title = $Matches[1]
            if (-not $startedTitles.ContainsKey($title)) {
              $startedTitles[$title] = Get-Date
              $i = $startedTitles.Count
              $source = $sources[$i - 1]
              if ($title -eq $titles.Noise) { $s2Started = Get-Date }
              $goFile = Join-Path $run "GO-$i"
              # Two independent references of the cable per session, outside the job: cpal and raw WASAPI.
              $refDir = Join-Path $run "reference-$i"; $refStop = Join-Path $run "STOP-REF-$i"
              $ref2Dir = Join-Path $run "reference2-$i"; $ref2Stop = Join-Path $run "STOP-REF2-$i"
              $refProc = Start-Process -FilePath $Testbench -ArgumentList @('capture', '--device', (Q $capName), '--dir', (Q $refDir), '--stop-file', (Q $refStop), '--go-file', (Q $goFile), '--report', (Q (Join-Path $run "reference-$i.json"))) -PassThru -WindowStyle Hidden -RedirectStandardOutput (Join-Path $run "reference-$i.out") -RedirectStandardError (Join-Path $run "reference-$i.err")
              $ref2Proc = Start-Process -FilePath $Testbench -ArgumentList @('capture-wasapi', '--device', (Q $capName), '--dir', (Q $ref2Dir), '--stop-file', (Q $ref2Stop), '--go-file', (Q $goFile), '--report', (Q (Join-Path $run "reference2-$i.json"))) -PassThru -WindowStyle Hidden -RedirectStandardOutput (Join-Path $run "reference2-$i.out") -RedirectStandardError (Join-Path $run "reference2-$i.err")
              Start-Sleep -Seconds 1
              $play = Start-Process -FilePath $Testbench -ArgumentList @('play', '--device', (Q $playName), '--file', (Q $source), '--go-file', (Q $goFile), '--report', (Q (Join-Path $run "play-$i.json"))) -PassThru -WindowStyle Hidden -RedirectStandardOutput (Join-Path $run "play-$i.out") -RedirectStandardError (Join-Path $run "play-$i.err")
              $rig = Set-WbRigPins -Plan $rigPlan -Player $play -Reference $refProc -GoFile $goFile
              if ($rigPlan -and -not $rigConfirmed) { $rig = Confirm-WbRigAgainstJob -Rig $rig -Player $play -Reference $refProc; $rigConfirmed = $true }
              $players += $play
              $refs += [pscustomobject]@{ Index = $i; Title = $title; Proc = $refProc; Stop = $refStop }
              $refs2 += [pscustomobject]@{ Index = $i; Title = $title; Proc = $ref2Proc; Stop = $ref2Stop }
              $progress.Add("$(Get-Date -Format 'HH:mm:ss') session $i started: $title; player and both reference captures started")
            }
          }
          if ($l -match 'DAY: stopped "(.+?)"') {
            $stoppedTitle = $Matches[1]
            foreach ($r in @($refs | Where-Object { $_.Title -eq $stoppedTitle })) { $refStopSeen[$r.Index] = Get-Date }
            if ($stoppedTitle -eq $titles.Noise) { $s2Stopped = Get-Date }
            $progress.Add("$(Get-Date -Format 'HH:mm:ss') $l")
          }
          if ($l -match 'every session of the plan was recorded') { $allRecorded = Get-Date; $progress.Add("$(Get-Date -Format 'HH:mm:ss') all sessions recorded") }
          $u = ConvertFrom-WbQueueUnitLine -Line $l -Slug $slug1
          if ($u) { $units1.Add([pscustomobject]@{ At = (Get-Date); Seconds = $u.Seconds; Unit = $u.Unit; Device = $u.Device }); $progress.Add("$(Get-Date -Format 'HH:mm:ss') $l") }
          if ($l -match ([regex]::Escape($slug1) + ': post-1 transcript complete')) { $s1Complete = Get-Date; $progress.Add("$(Get-Date -Format 'HH:mm:ss') $l") }
        }
        $lines = $ref.Value
        # Each session is stopped 15 s after its player has finished; each reference capture 3 s after the app's stop line.
        for ($pi = 0; $pi -lt $players.Count; $pi++) {
          $n = $pi + 1
          if ($stopWritten.ContainsKey($n)) { continue }
          if (-not $playerExit.ContainsKey($n) -and $players[$pi].HasExited) { $playerExit[$n] = Get-Date; $progress.Add("$(Get-Date -Format 'HH:mm:ss') player $n has finished playing") }
          $exitedAt = $null
          if ($playerExit.ContainsKey($n)) { $exitedAt = $playerExit[$n] }
          if (Test-WbDaySessionStopDue -PlayerExitedAt $exitedAt -Now (Get-Date) -MarginSeconds (Get-WbDayStopMarginSeconds)) {
            Set-Content -LiteralPath (Join-Path $run "STOP-SESSION-$n") -Value 'stop' -Encoding ascii
            $stopWritten[$n] = Get-Date
          }
        }
        foreach ($r in $refs) {
          if ($refStopWritten.ContainsKey($r.Index)) { continue }
          $seen = $null
          if ($refStopSeen.ContainsKey($r.Index)) { $seen = $refStopSeen[$r.Index] }
          if (Test-WbDayRefStopDue -AppStoppedSeenAt $seen -Now (Get-Date) -DelaySeconds (Get-WbDayRefStopDelaySeconds)) {
            Set-Content -LiteralPath $r.Stop -Value 'stop' -Encoding ascii
            $r2 = @($refs2 | Where-Object { $_.Index -eq $r.Index })[0]
            if ($r2) { Set-Content -LiteralPath $r2.Stop -Value 'stop' -Encoding ascii }
            $refStopWritten[$r.Index] = Get-Date
          }
        }
        # Once both sessions are recorded the queue gets time to finish session 1; then the app is told to stop.
        if ($allRecorded -and -not $dayStopAt) {
          $waited = ((Get-Date) - $allRecorded).TotalMinutes
          if ($s1Complete -or $waited -ge $settings.DecodeWaitMinutes) {
            Start-Sleep -Seconds 5
            Set-Content -LiteralPath $dayStop -Value 'stop' -Encoding ascii
            $dayStopAt = Get-Date
            $progress.Add("$(Get-Date -Format 'HH:mm:ss') stop file of the day written ($(if ($s1Complete) { 'session 1 is transcribed' } else { "session 1 was not transcribed after $($settings.DecodeWaitMinutes) minutes" }))")
          }
        }
        Start-Sleep -Seconds 1
      }
      # What is left of the log after the app exited.
      $ref = [ref]$lines
      foreach ($l in (Read-NewLines $queueLog $ref)) {
        $u = ConvertFrom-WbQueueUnitLine -Line $l -Slug $slug1
        if ($u) { $units1.Add([pscustomobject]@{ At = (Get-Date); Seconds = $u.Seconds; Unit = $u.Unit; Device = $u.Device }) }
      }
      foreach ($r in @($refs) + @($refs2)) { if (-not (Test-Path -LiteralPath $r.Stop)) { Set-Content -LiteralPath $r.Stop -Value 'stop' -Encoding ascii }; [void]$r.Proc.WaitForExit(120000) }
      $finished = $true
    } finally {
      if (-not $finished) {
        $left = @(Stop-WbRigLeftovers -Process (@($emu) + $players + @($refs | ForEach-Object { $_.Proc }) + @($refs2 | ForEach-Object { $_.Proc })) -StopFile (@($dayStop) + @($refs | ForEach-Object { $_.Stop }) + @($refs2 | ForEach-Object { $_.Stop })) -GraceSeconds 45)
        foreach ($l in $left) { Write-Host "cleanup after an error: $l" }
      }
    }
    $endedAt = Get-Date

    # ---- judging ----
    $report = Read-Json $reportFile
    if (-not $report -or -not $report.sessions) { throw "the app wrote no report of the sessions (see $reportFile)" }
    $s1 = @(@($report.sessions) | Where-Object { $_.title -eq $titles.Speech })[0]
    $s2 = @(@($report.sessions) | Where-Object { $_.title -eq $titles.Noise })[0]
    if (-not $s1 -or -not $s2) { throw "the app recorded $(@($report.sessions).Count) session(s), not both of the plan (see $reportFile)" }
    $j1 = Get-SessionJudging -Index 1 -Title $titles.Speech -Source $speechSrc -Noise $false -SessionReport $s1 -Run $run
    $j2 = Get-SessionJudging -Index 2 -Title $titles.Noise -Source $noiseSrc -Noise $true -SessionReport $s2 -Run $run
    $slugDir = Split-Path -Leaf ([string]$s1.dir)
    $job = Get-WbQueueSoakJob -Report $report -Slug $slugDir -Revision 'post-1'
    $jobCheck = Test-WbQueueSoakJobComplete -Job $job
    $segFile = Join-Path ([string]$s1.dir) 'post-1.jsonl'
    $segs = @()
    if (Test-Path -LiteralPath $segFile) { $segs = @(Get-Content -LiteralPath $segFile -Encoding UTF8 | Where-Object { $_.Trim() } | ForEach-Object { $_ | ConvertFrom-Json }) }
    $audioMs = $(if ($job) { [double]$job.audio_ms } else { $j1.AudioMs })
    $segCheck = Test-WbQueueSoakSegments -Segments $segs -AudioMs $audioMs
    $overlapS = 0.0
    if ($s2Started -and $s2Stopped) { $overlapS = Get-WbDecodeOverlapSeconds -Units @($units1.ToArray()) -WindowStart $s2Started -WindowEnd $s2Stopped }
    $logLines = @()
    if (Test-Path -LiteralPath $queueLog) { $logLines = @(Get-Content -LiteralPath $queueLog -Encoding UTF8) }
    $logInfo = Get-WbQueueSoakLogInfo -Lines $logLines -Slug $slugDir
    $verdict = Get-WbQueueSoakVerdict -Recording1 $j1.Verdict -Recording2 $j2.Verdict -Segments $segCheck -JobCheck $jobCheck -OverlapSeconds $overlapS
    $appLogLines = @($progress.ToArray())
    [pscustomobject]@{
      NotApplicable = $false; Run = $run; Verdict = $verdict; J1 = $j1; J2 = $j2; SegCheck = $segCheck; Job = $job; JobCheck = $jobCheck; OverlapS = $overlapS
      LogInfo = $logInfo; Progress = $appLogLines; Settings = $settings; CableRig = $cableRig; CableTest = $cableTest
      StartedAt = $startedAt; EndedAt = $endedAt; AllRecorded = $allRecorded; S1Complete = $s1Complete; S2Started = $s2Started; S2Stopped = $s2Stopped
      Report = $report; Decision = $decision; Units1 = @($units1.ToArray()); DecodeWaitedOut = [bool](-not $s1Complete)
    }
  }
} catch {
  $detail = $(if ($script:Current -and $script:Current.Error) { $script:Current.Error } else { ($_.Exception.GetType().FullName + ': ' + $_.Exception.Message + "`r`n" + $_.ScriptStackTrace) })
  Write-WbGateCrashEvidence -Path (Join-Path $OutDir "$gateName-$Profile.txt") -Reason "the $gateName gate stopped on an error: $($_.Exception.Message)" -Detail $detail -Machine $Machine -Profile $Profile -Builds $builds
  Write-Host "FAIL: the $gateName gate did not complete: $($_.Exception.Message)"
  if ($_.Exception.Message -match '^(NEEDS A SYSTEM CHECK|RIG REFUSED|RIG SELF-TEST FAILED|Overlay tools are running|Only [0-9.,]+ GB)') { exit 3 }
  exit 1
}

# ---- verdict and evidence ----
$r = @($result)[-1]
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$evidence = Join-Path $OutDir "$gateName-$Profile.txt"
$lines = @()
$lines += Get-WbGateHeader -Machine $Machine -Profile $Profile -Builds $builds
$lines += "app build: target\gate\stelmar-voice-workbench.exe (cargo profile gate, features gate-hooks + custom-protocol)"
if ($r.NotApplicable) {
  $lines += 'verdict: NOT APPLICABLE (this machine''s system check did not clear processing while recording, so the app''s queue correctly does not transcribe during a recording)'
  $lines += ''
  $lines += "system check of this pack run: tier $($r.Decision.tier); process_while_recording false. Nothing was recorded."
  $lines += ''
  $lines += "audio and reports: $($r.Run) (not in the repository)"
  Write-WbEvidence -Path $evidence -Lines $lines -MachineLabel $Machine
  Write-Host "NOT APPLICABLE: $evidence"
  exit 4
}
$v = $r.Verdict
$status = $v.Status
$durMin = ($r.EndedAt - $r.StartedAt).TotalMinutes
$verdictText = "verdict: $(if ($status -eq 'INVALID') { 'INVALID (no verdict; the run must be repeated): ' + ($v.Invalid -join '; ') } elseif ($status -eq 'PASS' -and $Profile -notin @('native', 'laptop')) { 'PASS (PROVISIONAL: emulated)' } elseif ($status -eq 'FAIL') { 'FAIL: ' + ($v.Fail -join '; ') } else { $status })"
$lines += $verdictText
$lines += ''
$lines += 'checks:'
foreach ($k in $v.Checks.Keys) { $lines += "  [$(if ($v.Checks[$k]) { 'x' } else { ' ' })] $k" }
$lines += ''
$lines += $r.J1.Lines
$lines += ''
$lines += $r.J2.Lines
$lines += ''
$lines += ('session 1 transcript: {0}' -f $(if ($r.SegCheck.Ok -and $r.JobCheck.Ok) { 'complete' } else { 'NOT complete' }))
foreach ($p in @($r.SegCheck.Problems)) { $lines += "  $p" }
if (-not $r.JobCheck.Ok) { $lines += "  $($r.JobCheck.Reason)" }
$lines += ''
$lines += Get-WbQueueSoakInfoLines -LogInfo $r.LogInfo -OverlapSeconds $r.OverlapS -Segments $r.SegCheck -Job $r.Job
$lines += ''
$lines += 'run:'
$lines += ('  mode: {0}; session 1 {1} s of public speech (FLEURS English and Hindi, CC BY 4.0), session 2 {2} s of seeded noise; part length {3}' -f $r.Settings.Mode, $Session1Seconds, $Session2Seconds, $(if ($PartSeconds -gt 0) { "$PartSeconds s" } else { 'the app default (10 min)' }))
$lines += ('  real duration of the gate: {0:n1} min (from the first check to the app''s exit); session 1 was {1} when the app was told to stop' -f $durMin, $(if ($r.DecodeWaitedOut) { 'NOT yet transcribed' } else { 'transcribed' }))
if ($r.AllRecorded -and $r.S1Complete) { $lines += ('  session 1 finished transcribing {0:n1} min after both sessions were recorded' -f (($r.S1Complete - $r.AllRecorded).TotalMinutes)) }
if ($RigPrefer) { $lines += Get-PackRigLines -Rig $r.CableRig } else { $lines += Get-WbCableRigLines -Rig $r.CableRig -SelfTest $r.CableTest }
$lines += ''
$lines += 'progress:'
foreach ($p in @($r.Progress)) { $lines += "  $p" }
$lines += ''
$lines += "audio and reports: $($r.Run) (not in the repository)"
Write-WbEvidence -Path $evidence -Lines $lines -MachineLabel $Machine
Write-Host "$status`: $evidence"
if ($status -eq 'INVALID') { exit 3 }
if ($status -eq 'FAIL') { exit 1 }
exit 0
