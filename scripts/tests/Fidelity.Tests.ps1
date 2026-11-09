# Tests for scripts/lib/Fidelity.psm1: the recorder-gate rule of 9 Nov 2026 (docs/PLAN.md section 12).
# Run: powershell -NoProfile -ExecutionPolicy Bypass -File scripts/tests/Fidelity.Tests.ps1
# ASCII only; Windows PowerShell 5.1.
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
Import-Module (Join-Path $here '..\lib\GateRun.psm1') -Force
Import-Module (Join-Path $here '..\lib\Fidelity.psm1') -Force
$fails = 0
function Check([string]$Name, [bool]$Ok) {
  if ($Ok) { Write-Host "ok   $Name" } else { Write-Host "FAIL $Name"; $script:fails++ }
}

$playAt = [DateTimeOffset]::Parse('2026-11-09T20:58:34.532+05:30')

function New-Compare {
  param([int64]$Differing = 0, [int64]$RefOnly = 0, [int64]$AppOnly = 0, [bool]$Aligned = $true)
  [pscustomobject]@{
    sample_rate = 48000; channels = 2; app_frames = 8000000; reference_frames = 8000000
    segments = @([pscustomobject]@{ index = 0; app_start_frame = 0; app_end_frame = 8000000; aligned = $Aligned; offset_frames = 48000; aligned_by = 'reference'; compared_frames = 7900000; differing_frames = $Differing; max_abs_diff = $(if ($Differing) { 1 } else { 0 }); differing_ranges = @(); app_only_frames = 100000; app_only_loud_frames = $AppOnly; reference_start_frame = 0; reference_end_frame = 7900000 })
    aligned = $Aligned; compared_frames = 7900000; differing_frames = $Differing
    reference_only_loud_frames = $RefOnly; app_only_loud_frames = $AppOnly; restart_gap_frames = 0
    equal = ($Aligned -and $Differing -eq 0 -and $RefOnly -eq 0); notes = @()
  }
}

# Reference-vs-source analysis for a run of $Minutes with the given breaks.
function New-RefAnalysis {
  param([double]$Minutes = 30, $Jumps = @(), $Unmatched = @(), $Xruns = @())
  [pscustomobject]@{
    sample_rate = 48000; recorded_ms = $Minutes * 60000.0; source_ms = $Minutes * 60000.0
    content_start_ms = 2278.0; content_first_source_frame = 0
    discontinuities = @($Jumps | ForEach-Object { [pscustomobject]@{ source_ms = $_[0]; source_jump_ms = $_[1]; source_jump_frames = [int64]($_[1] * 48) } })
    unmatched_ranges = @($Unmatched | ForEach-Object { [pscustomobject]@{ source_ms = $_[0]; ms = $_[1] } })
    xruns = @($Xruns)
  }
}

function New-Play {
  param([double]$MaxGap = 21.9, $LateEvents = @())
  [pscustomobject]@{ underrun_callbacks = 0; late_callbacks = @($LateEvents).Count; max_callback_interval_ms = $MaxGap; late_callback_events = @($LateEvents) }
}

$ref = [pscustomobject]@{ overrun_samples = 0; xruns = 1 }

function Score($compare, $analysis, $play, $reference = $ref, [int64]$dropped = 0) {
  Get-WbRunVerdict -Compare $compare -RefAnalysis $analysis -Play $play -Reference $reference -PlayStarted $playAt -DroppedSamples $dropped
}

# ---- 1. A clean pass ---------------------------------------------------------------------------
$v = Score (New-Compare) (New-RefAnalysis) (New-Play)
Check 'a clean run with equal recordings and no breaks is PASS' ($v.Verdict -eq 'PASS' -and $v.InvalidReasons.Count -eq 0 -and $v.FailReasons.Count -eq 0 -and $v.Breaks.Count -eq 0)

# ---- 2. A fidelity FAIL: one sample changed in the app's recording -------------------------------
$v = Score (New-Compare -Differing 1) (New-RefAnalysis) (New-Play)
Check 'one differing frame between the app and the reference is FAIL' ($v.Verdict -eq 'FAIL' -and ($v.FailReasons -join ' ') -match 'differs from the reference capture in 1 frame')
$v = Score (New-Compare -RefOnly 480) (New-RefAnalysis) (New-Play)
Check 'audio the reference holds and the app has no recording of is FAIL' ($v.Verdict -eq 'FAIL' -and ($v.FailReasons -join ' ') -match '480 frame')
$v = Score (New-Compare) (New-RefAnalysis) (New-Play) $ref 960
Check 'dropped samples are a FAIL even when the recordings are equal' ($v.Verdict -eq 'FAIL' -and ($v.FailReasons -join ' ') -match 'dropped 960 sample')

# ---- 3. A valid upstream break: reported, not gated ------------------------------------------------
$xr = @([pscustomobject]@{ n = 1; callbacks_before = 0; frames_before = 0; audio_ms = 0.0; wall_clock = '2026-11-09T20:58:34.532+05:30' })
$a = New-RefAnalysis -Minutes 30 -Jumps @(,@(52000.0, -10.4)) -Unmatched @(,@(51978.7, 21.3)) -Xruns $xr
$v = Score (New-Compare) $a (New-Play)
Check 'an upstream break inside the limits leaves the verdict PASS' ($v.Verdict -eq 'PASS' -and $v.Breaks.Count -eq 1)
$b = $v.Breaks[0]
Check 'the repeat and the unmatched stretch beside it are one break' ([math]::Abs($b.SourceMs - 51978.7) -lt 0.01 -and [math]::Abs($b.RepeatMs - 10.4) -lt 0.01 -and [math]::Abs($b.UnmatchedMs - 21.3) -lt 0.01)
Check 'the break has its wall-clock time and the nearest reference xrun' ($b.Wall.ToString('HH:mm:ss') -eq '20:59:26' -and $b.NearestXrun.N -eq 1)
$text = (Get-WbVerdictLines -Result $v -Compare (New-Compare)) -join "`n"
Check 'the evidence lists the break with its position, wall clock, lengths and xrun' ($text -match 'at source 51.979 s, 20:59:26\.\d+ IST: repeat 10\.4 ms, skipped 0\.0 ms, unmatched 21\.3 ms; nearest reference xrun #1' -and $text -match 'late player callbacks within 500 ms: 0')
$two = New-RefAnalysis -Minutes 30 -Jumps @(@(1000.0, -10.0), @(90000.0, -12.0)) -Unmatched @(@(990.0, 20.0), @(89980.0, 22.0))
Check 'breaks far apart are separate breaks' ((Score (New-Compare) $two (New-Play)).Breaks.Count -eq 2)

# ---- 4. INVALID for each limit and each missing piece ------------------------------------------------
$late = @([pscustomobject]@{ source_ms = 52200.0; at_ms = 53000.0; interval_ms = 160.0; buffer_ms = 100.0 })
$v = Score (New-Compare) $a (New-Play -MaxGap 160.0 -LateEvents $late)
Check 'a late player callback within 500 ms of a break is INVALID' ($v.Verdict -eq 'INVALID' -and ($v.InvalidReasons -join ' ') -match 'late player callback')
$lateFar = @([pscustomobject]@{ source_ms = 90000.0; at_ms = 91000.0; interval_ms = 160.0; buffer_ms = 100.0 })
$v = Score (New-Compare) $a (New-Play -MaxGap 160.0 -LateEvents $lateFar)
Check 'a late callback far from any break does not make the run INVALID' ($v.Verdict -eq 'PASS')
$big = New-RefAnalysis -Minutes 30 -Jumps @(,@(52000.0, -10.4)) -Unmatched @(,@(51978.7, 251.0))
$v = Score (New-Compare) $big (New-Play)
Check 'a break with more than 250 ms unmatched is INVALID' ($v.Verdict -eq 'INVALID' -and ($v.InvalidReasons -join ' ') -match '251 ms unmatched')
$ok250 = New-RefAnalysis -Minutes 30 -Jumps @(,@(52000.0, -10.4)) -Unmatched @(,@(51978.7, 250.0))
Check 'exactly 250 ms unmatched is still inside the limit' ((Score (New-Compare) $ok250 (New-Play)).Verdict -eq 'PASS')
$many = New-RefAnalysis -Minutes 20 -Jumps @(@(5000.0, -10.0), @(90000.0, -12.0), @(500000.0, -11.0)) -Unmatched @()
$v = Score (New-Compare) $many (New-Play)
Check 'more than max(1, floor(minutes / 10)) counted upstream breaks is INVALID' ($v.Verdict -eq 'INVALID' -and ($v.InvalidReasons -join ' ') -match '3 counted upstream break' -and ($v.InvalidReasons -join ' ') -match 'limit is 2')
$twoIn20 = New-RefAnalysis -Minutes 20 -Jumps @(@(1000.0, -10.0), @(90000.0, -12.0)) -Unmatched @()
Check 'a 20-minute run allows 2 counted breaks' ((Score (New-Compare) $twoIn20 (New-Play)).Verdict -eq 'PASS')
$short = New-RefAnalysis -Minutes 3 -Jumps @(,@(3877.0, -10.96)) -Unmatched @(,@(3877.0, 22.7))
$v = Score (New-Compare) $short (New-Play)
Check 'Amendment 1 (b): a single break in a 3-minute run is inside the limit of 1 (it was INVALID as a rate)' ($v.Verdict -eq 'PASS' -and $v.Breaks.Count -eq 1)
Check 'a reference capture that is missing is INVALID' ((Score (New-Compare) $null (New-Play) $null).Verdict -eq 'INVALID')
Check 'an app recording that was not compared is INVALID' ((Score $null (New-RefAnalysis) (New-Play)).Verdict -eq 'INVALID')
$v = Score (New-Compare -Aligned $false) (New-RefAnalysis) (New-Play)
Check 'a recording that cannot be aligned with one fixed offset is INVALID, not FAIL' ($v.Verdict -eq 'INVALID' -and ($v.InvalidReasons -join ' ') -match 'misaligned')
$v = Score (New-Compare -AppOnly 5000) (New-RefAnalysis) (New-Play)
Check 'audio the app recorded that the reference lacks (incomplete reference) is INVALID' ($v.Verdict -eq 'INVALID' -and ($v.InvalidReasons -join ' ') -match 'incomplete reference')
$v = Score (New-Compare) (New-RefAnalysis) (New-Play) ([pscustomobject]@{ overrun_samples = 480; xruns = 0 })
Check 'a reference capture that lost samples itself is INVALID' ($v.Verdict -eq 'INVALID' -and ($v.InvalidReasons -join ' ') -match 'lost samples itself')
$noLate = [pscustomobject]@{ underrun_callbacks = 0 }
$v = Score (New-Compare) $a $noLate
Check 'Amendment 1 (a): a run with an upstream break and no late-callback data is INVALID, not PASS' ($v.Verdict -eq 'INVALID' -and ($v.InvalidReasons -join ' ') -match 'upstream break and the player recorded no late-callback data')
$v = Score (New-Compare) (New-RefAnalysis) $noLate
Check 'Amendment 1 (a): a run with 0 upstream breaks needs no late-callback data' ($v.Verdict -eq 'PASS' -and $v.Breaks.Count -eq 0)
$v = Score (New-Compare) $a (New-Play -MaxGap 21.9)
Check 'Amendment 1 (a): the same break with late-callback data is attributed and the run is valid' ($v.Verdict -eq 'PASS')
$v = Score (New-Compare -Differing 5) (New-RefAnalysis -Minutes 3 -Jumps @(,@(3877.0, -10.96)) -Unmatched @(,@(3877.0, 300.0))) (New-Play)
Check 'INVALID takes precedence over a fidelity FAIL (the run is repeated)' ($v.Verdict -eq 'INVALID' -and $v.FailReasons.Count -eq 1)
$text = (Get-WbVerdictLines -Result $v -Compare (New-Compare -Differing 5)) -join "`n"
Check 'the evidence says INVALID needs a repeat and still lists the fidelity result' ($text -match 'validity: INVALID \(no verdict; the run must be repeated\)' -and $text -match 'fidelity FAIL')

# ---- Amendment 1 (b): the limit is max(1, floor(minutes / 10)) counted breaks per run -------------------
function Breaks($n, $firstMs = 5000.0) { 1..$n | ForEach-Object { , @(($firstMs + ($_ * 20000.0)), -10.0) } }
Check '3 minutes: 1 counted break is inside the limit, 2 are INVALID' ((Score (New-Compare) (New-RefAnalysis -Minutes 3 -Jumps @(Breaks 1)) (New-Play)).Verdict -eq 'PASS' -and (Score (New-Compare) (New-RefAnalysis -Minutes 3 -Jumps @(Breaks 2)) (New-Play)).Verdict -eq 'INVALID')
Check '19.9 minutes still allows only 1 counted break' ((Score (New-Compare) (New-RefAnalysis -Minutes 19.9 -Jumps @(Breaks 1)) (New-Play)).Verdict -eq 'PASS' -and (Score (New-Compare) (New-RefAnalysis -Minutes 19.9 -Jumps @(Breaks 2)) (New-Play)).Verdict -eq 'INVALID')
Check '30 minutes allows 3 counted breaks, not 4' ((Score (New-Compare) (New-RefAnalysis -Minutes 30 -Jumps @(Breaks 3)) (New-Play)).Verdict -eq 'PASS' -and (Score (New-Compare) (New-RefAnalysis -Minutes 30 -Jumps @(Breaks 4)) (New-Play)).Verdict -eq 'INVALID')
Check '120 minutes allows 12 counted breaks, not 13' ((Score (New-Compare) (New-RefAnalysis -Minutes 120 -Jumps @(Breaks 12)) (New-Play)).Verdict -eq 'PASS' -and (Score (New-Compare) (New-RefAnalysis -Minutes 120 -Jumps @(Breaks 13)) (New-Play)).Verdict -eq 'INVALID')

# ---- Amendment 1 (c): breaks in the first 3 s of playback are reported, not counted --------------------
$early = New-RefAnalysis -Minutes 3 -Jumps @(@(17.0, -1.6), @(1798.0, -12.08), @(60000.0, -10.0)) -Unmatched @(@(17.0, 82.7), @(1706.0, 92.0))
$v = Score (New-Compare) $early (New-Play)
Check 'two start-up breaks plus one later break in a 3-minute run: all reported, 1 counted, valid' ($v.Verdict -eq 'PASS' -and $v.Breaks.Count -eq 3 -and $v.CountedBreaks.Count -eq 1)
$two = New-RefAnalysis -Minutes 3 -Jumps @(@(17.0, -1.6), @(60000.0, -10.0), @(100000.0, -10.0))
Check 'start-up breaks do not make room: 2 later breaks in 3 minutes are still INVALID' ((Score (New-Compare) $two (New-Play)).Verdict -eq 'INVALID')
Check 'a break at 2,999 ms is not counted and one at 3,000 ms is' ((Score (New-Compare) (New-RefAnalysis -Minutes 3 -Jumps @(,@(2999.0, -10.0))) (New-Play)).CountedBreaks.Count -eq 0 -and (Score (New-Compare) (New-RefAnalysis -Minutes 3 -Jumps @(,@(3000.0, -10.0))) (New-Play)).CountedBreaks.Count -eq 1)
$earlyBig = New-RefAnalysis -Minutes 3 -Jumps @(,@(1706.0, -12.0)) -Unmatched @(,@(1706.0, 260.0))
$v = Score (New-Compare) $earlyBig (New-Play)
Check 'a start-up break with more than 250 ms unmatched is still INVALID' ($v.Verdict -eq 'INVALID' -and ($v.InvalidReasons -join ' ') -match '260 ms unmatched')
$lateEarly = @([pscustomobject]@{ source_ms = 1900.0; at_ms = 3000.0; interval_ms = 150.0; buffer_ms = 100.0 })
$v = Score (New-Compare) (New-RefAnalysis -Minutes 3 -Jumps @(,@(1706.0, -12.0))) (New-Play -MaxGap 150.0 -LateEvents $lateEarly)
Check 'a start-up break with a late player callback within 500 ms is still INVALID' ($v.Verdict -eq 'INVALID' -and ($v.InvalidReasons -join ' ') -match 'late player callback')
$v = Score (New-Compare) (New-RefAnalysis -Minutes 3 -Jumps @(,@(1706.0, -12.0))) $noLate
Check 'a start-up break without late-callback data is still INVALID (it cannot be attributed)' ($v.Verdict -eq 'INVALID')

# ---- the amendment leaves the fidelity check alone ------------------------------------------------------
Check 'fidelity still decides FAIL with breaks inside the new limits' ((Score (New-Compare -Differing 1) (New-RefAnalysis -Minutes 3 -Jumps @(Breaks 1)) (New-Play)).Verdict -eq 'FAIL')
Check 'dropped samples still FAIL with no breaks and no late-callback data' ((Score (New-Compare) (New-RefAnalysis) $noLate $ref 480).Verdict -eq 'FAIL')

# ---- 5. The first, too sensitive late-callback data is judged by the buffer ---------------------------
$oldFormat = [pscustomobject]@{
  underrun_callbacks = 0; late_callbacks = 120; max_callback_interval_ms = 21.5
  late_callback_events = @([pscustomobject]@{ source_ms = 3880.0; at_ms = 5000.0; interval_ms = 20.0; nominal_ms = 10.0 })
}
$l = Get-WbPlayerLateCallbacks -Play $oldFormat
Check 'data from the 1.5-times threshold with a longest gap under the buffer holds no late callback' ($l.Known -and $l.Events.Count -eq 0)

if ($fails) { Write-Host "$fails failed"; exit 1 }
Write-Host 'Fidelity tests passed'
