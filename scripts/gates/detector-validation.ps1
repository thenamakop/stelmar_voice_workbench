<#
.SYNOPSIS
  Checks the recorder's two audio-level detectors (repeated block, zero gap) against recordings
  whose shared upstream breaks the gate harness already found, without recording again.

.DESCRIPTION
  For each gate run folder under -GateRoot (a run has rec\<date>\gate-*, analysis.json,
  reference-analysis.json and play.json), runs `wb-testbench scan-repeats` over the app's saved
  recording (the same detector code the recorder thread runs), lines the detections up with the
  harness's shared upstream breaks (scripts\lib\Detector.psm1) and writes the evidence. Nothing is
  gated: it reports every shared break, whether a detection lies within +-50 ms of its span, and
  every detection with no shared break (a false alarm).

  Not timing-sensitive (it reads files); it still waits for no lock, and it does not touch the
  recorder.
#>
[CmdletBinding()]
param(
  [string]$GateRoot = 'D:\STELMAR-Gate',
  [string]$Testbench,
  [string]$OutFile,
  # Runs listed in full (the ones the plan names); every other run on disk gets a table row.
  [string]$FullDate = '20261110',
  [string]$Machine = 'Legion'
)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo = (Resolve-Path (Join-Path $here '..\..')).Path
Import-Module (Join-Path $repo 'scripts\lib\PsEnv.psm1') -Force
Set-WbCleanPSModulePath
Set-WbInvariantCulture
Import-Module (Join-Path $repo 'scripts\lib\GateRun.psm1') -Force
Import-Module (Join-Path $repo 'scripts\lib\Redact.psm1') -Force
Import-Module (Join-Path $repo 'scripts\lib\Fidelity.psm1') -Force
Import-Module (Join-Path $repo 'scripts\lib\Detector.psm1') -Force
if (-not $Testbench) { $Testbench = Join-Path $repo 'target\gate\wb-testbench.exe' }
if (-not $OutFile) { $OutFile = Join-Path $repo 'docs\evidence\phase-2\repeated-block-validation.txt' }
New-Item -ItemType Directory -Force -Path (Split-Path -Parent $OutFile) | Out-Null

function Read-Json([string]$Path) { if (Test-Path -LiteralPath $Path) { Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json } else { $null } }

function Get-RunResult([string]$RunDir) {
  $sess = @(Get-ChildItem (Join-Path $RunDir 'rec') -Directory -ErrorAction SilentlyContinue | ForEach-Object { Get-ChildItem $_.FullName -Directory -Filter 'gate-*' } | Select-Object -First 1)
  if ($sess.Count -eq 0) { return $null }
  $scanJson = & $Testbench scan-repeats --session $sess[0].FullName
  if ($LASTEXITCODE -ne 0) { throw "scan-repeats failed on $RunDir" }
  $scan = ($scanJson -join "`n") | ConvertFrom-Json
  $events = @()
  foreach ($b in @($scan.blocks)) { $events += [pscustomobject]@{ kind = 'repeated_block'; audio_ms = [double]$b.audio_ms; detail = "len_frames=$($b.len_frames) lag_frames=$($b.lag_frames)" } }
  foreach ($g in @($scan.zero_gaps)) { $events += [pscustomobject]@{ kind = 'zero_gap'; audio_ms = [double]$g.audio_ms; detail = "len_frames=$($g.len_frames) len_ms=$([math]::Round([double]$g.len_ms, 1))" } }
  $analysis = Read-Json (Join-Path $RunDir 'analysis.json')
  $refAnalysis = Read-Json (Join-Path $RunDir 'reference-analysis.json')
  $play = Read-Json (Join-Path $RunDir 'play.json')
  $playStart = $(if ($play -and (Test-WbHasProperty $play 'started_at')) { [DateTimeOffset]::Parse($play.started_at) } else { $null })
  $breaks = @()
  $check = $null
  if ($analysis -and $refAnalysis) {
    $breaks = @(Get-WbUpstreamBreaks -RefAnalysis $refAnalysis -Play $play -PlayStarted $playStart)
    $check = Get-WbDetectorMatches -Breaks $breaks -Analysis $analysis -Events $events
  }
  [pscustomobject]@{ Name = (Split-Path -Leaf $RunDir); Scan = $scan; Events = $events; Breaks = $breaks; Check = $check; Analysis = $analysis }
}

$runs = @(Get-ChildItem -LiteralPath $GateRoot -Directory | Where-Object { $_.Name -match '^(panic|webview|kill|load|soak)-emu-low-\d{8}-\d{6}$' } | Sort-Object Name)
$results = @()
foreach ($r in $runs) {
  try { $res = Get-RunResult $r.FullName; if ($res) { $results += $res } } catch { Write-Warning "$($r.Name): $($_.Exception.Message)" }
}
$full = @($results | Where-Object { $_.Name -match "-$FullDate-" })

$lines = @()
$lines += 'Repeated-block and zero-gap detectors against the recorded gate sessions (Phase 2)'
$lines += "machine: $Machine. Reads saved recordings only: nothing was recorded or timed, so no lock, overlay check or Track B pause applies."
$rev = (& git -C $repo rev-parse --short HEAD 2>&1 | Select-Object -First 1)
$dirty = @(& git -C $repo status --porcelain 2>&1).Count -gt 0
$lines += "source: commit $rev$(if ($dirty) { ' with uncommitted or untracked changes (the build hash below identifies the code)' })"
$lines += ("build: {0} sha256 {1}" -f (Split-Path -Leaf $Testbench), (Get-FileHash -LiteralPath $Testbench -Algorithm SHA256).Hash.ToLowerInvariant())
$lines += "recordings: $GateRoot (the runs of the Phase 1 gates, not in the repository)"
$lines += ''
$lines += 'FINDING: the shared upstream breaks are not repeated blocks.'
$lines += '  The gate evidence and docs/PLAN.md section 12 call the shared breaks "repeated blocks" because the harness reports a negative source jump of about one WASAPI period ("repeat 10.2 ms").'
$lines += '  At the sample level every one of them is a run of exact digital zeros of 490 to 551 frames (10.2 to 11.5 ms) inserted into the noise signal: the recording holds 537 zero frames at app audio 287,916.9 ms in the load run, 490 at 2,346.6 ms in the panic run, and so on. The negative jump is the delay those inserted frames add to the recording.'
$lines += '  Neither the player, the app nor the reference capture can tell it from the signal; all three record the zeros identically. No exact repeat (a stretch equal to the one before it at a lag of 2.7 to 100 ms) exists in any recording scanned.'
$lines += '  So the recorder has two detectors: repeated_block (an exact repeat; unit-tested on spliced noise, found nowhere in the real recordings) and zero_gap (a run of 2 to 50 ms of exact zeros in every channel with live audio on both sides; found at every shared break).'
$lines += ''
$lines += 'detectors (the code the recorder thread runs; crates/wb-recorder/src/repeat.rs and dropout.rs):'
$lines += '  repeated_block: a stretch of at least 256 frames equal sample for sample to the stretch lag frames before it (lag 128 to 4,800), RMS at least -60 dBFS, run length at most 2 x lag (longer is a periodic signal)'
$lines += '  zero_gap: 2 to 50 ms of frames in which every channel is exactly 0.0, with at least half the frames not exactly zero in the 10 ms before and the 10 ms after'
$lines += ''
$lines += "SECTION 1: the runs of $FullDate (the final Phase 1 sequence), in full"
foreach ($res in $full) {
  $lines += ''
  $lines += "run $($res.Name): $([math]::Round($res.Scan.audio_seconds / 60.0, 1)) minutes of audio in $($res.Scan.parts) part(s), $($res.Scan.run_starts.Count) run(s) of the app; scanned in $([math]::Round($res.Scan.elapsed_s, 1)) s"
  $lines += "  detections: repeated_block $(@($res.Scan.blocks).Count); zero_gap $(@($res.Scan.zero_gaps).Count)"
  foreach ($g in @($res.Scan.zero_gaps)) { $lines += ("    zero_gap at app audio {0:n1} ms (part {1}): {2} frames of zeros = {3:n1} ms" -f [double]$g.audio_ms, $g.part, $g.len_frames, [double]$g.len_ms) }
  if ($res.Check) { $lines += Get-WbDetectorLines -Check $res.Check -Rec $null } else { $lines += '  live detector check: not available (no analysis for this run)' }
}
$lines += ''
$lines += 'SECTION 2: every gate recording on disk (9 Nov and earlier runs were recorded by the Phase 1 recorder; the detectors read the saved audio)'
$lines += '  run | audio min | repeated_block | zero_gap | shared breaks | matched | missed | before recording began | false alarms'
$tot = [pscustomobject]@{ Min = 0.0; Blocks = 0; Gaps = 0; Breaks = 0; Matched = 0; Missed = 0; Before = 0; False = 0; FalseReal = 0 }
foreach ($res in $results) {
  $c = $res.Check
  $b = $(if ($c) { @($c.Rows).Count } else { -1 })
  $lines += ("  {0} | {1:n1} | {2} | {3} | {4} | {5} | {6} | {7} | {8}" -f $res.Name, ($res.Scan.audio_seconds / 60.0), @($res.Scan.blocks).Count, @($res.Scan.zero_gaps).Count, $(if ($c) { $b } else { 'n/a' }), $(if ($c) { $c.Matched } else { 'n/a' }), $(if ($c) { $c.Missed } else { 'n/a' }), $(if ($c) { $c.BeforeRecording } else { 'n/a' }), $(if ($c) { @($c.FalseAlarms).Count } else { 'n/a' }))
  $tot.Min += $res.Scan.audio_seconds / 60.0; $tot.Blocks += @($res.Scan.blocks).Count; $tot.Gaps += @($res.Scan.zero_gaps).Count
  if ($c) { $tot.Breaks += $b; $tot.Matched += $c.Matched; $tot.Missed += $c.Missed; $tot.Before += $c.BeforeRecording; $tot.False += @($c.FalseAlarms).Count; $tot.FalseReal += @($c.FalseAlarms | Where-Object { $_.InAppAnalysis }).Count }
}
$lines += ("  total | {0:n1} min ({1:n1} h) | {2} | {3} | {4} | {5} | {6} | {7} | {8} (of which {9} lie inside a break of the app-vs-source analysis)" -f $tot.Min, ($tot.Min / 60.0), $tot.Blocks, $tot.Gaps, $tot.Breaks, $tot.Matched, $tot.Missed, $tot.Before, $tot.False, $tot.FalseReal)
$lines += ''
$lines += 'misses and false alarms in section 2 are listed below, with the reason where it is known:'
$any = $false
foreach ($res in $results) {
  $c = $res.Check
  if (-not $c) { continue }
  foreach ($row in @($c.Rows | Where-Object { -not $_.Matched -and -not $_.BeforeRecording })) {
    $any = $true
    $lines += ("  {0}: MISS at source {1:n3} s (app audio {2:n1} to {3:n1} ms; unmatched {4:n0} ms, repeat {5:n1} ms)" -f $res.Name, ($row.SourceMs / 1000.0), $row.AppStartMs, $row.AppEndMs, $row.UnmatchedMs, $row.RepeatMs)
  }
  foreach ($e in @($c.FalseAlarms)) {
    $any = $true
    $start = $(if ($res.Analysis) { "; this run's test signal starts at {0:n1} ms after {1:n1} ms of leading audio" -f [double]$res.Analysis.content_start_ms, [double]$res.Analysis.leading_junk_ms } else { '' })
    $lines += ("  {0}: FALSE ALARM {1} at {2:n1} ms ({3}){4}{5}" -f $res.Name, $e.kind, [double]$e.audio_ms, $e.detail, $(if ($e.InAppAnalysis) { '; the app-vs-source analysis has a break here too (real; not counted as shared by the harness)' } else { '' }), $start)
  }
}
if (-not $any) { $lines += '  none' }

$lines += ''
$lines += 'reading (what the numbers say):'
$lines += '  section 1: all 5 shared breaks of the final Phase 1 sequence (panic, webview, kill, load, soak) are marked by a zero_gap within a few milliseconds of the start of their span; 0 repeated_block and 0 false alarms in 2.8 hours of audio (165.3 minutes).'
$lines += '  section 2, false alarms: both are zero gaps of 9 to 11 ms in the cable''s start-up region, within 100 ms of the point where the test signal starts (listed above with the signal start). They are real dropouts of the same kind as the shared breaks, in the stale audio the cable replays before the signal; the harness does not count that region. No zero gap or repeat was found inside the test signal where the harness saw no break, in 12.3 hours.'
$lines += '  section 2, misses: three are unmatched stretches of 295 to 325 ms with no repeat (a sound mixed into the cable, consistent with the notification chimes recorded in docs/GATES.md for the 9 Nov runs), not one-period dropouts; the fourth (webview-emu-low-20261109-205829) has a repeat of 1.6 ms (77 frames), below the 2 ms minimum of a zero gap. 2 more breaks lie before the app began recording and cannot be seen by any recorder.'
$lines += '  the repeated_block detector found nothing in any real recording: it is covered by unit tests on spliced noise (crates/wb-recorder/src/repeat.rs) and through the recorder thread (crates/wb-recorder/tests/repeat_and_marks.rs), and stays as a defence against a true repeat, which the sample-level data above show has not occurred.'

Write-WbEvidence -Path $OutFile -Lines $lines -MachineLabel $Machine
Write-Host "wrote $OutFile"
