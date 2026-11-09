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
$many = New-RefAnalysis -Minutes 20 -Jumps @(@(1000.0, -10.0), @(90000.0, -12.0), @(500000.0, -11.0)) -Unmatched @()
$v = Score (New-Compare) $many (New-Play)
Check 'more than 1 upstream break per 10 minutes of audio is INVALID' ($v.Verdict -eq 'INVALID' -and ($v.InvalidReasons -join ' ') -match '3 upstream break')
$twoIn20 = New-RefAnalysis -Minutes 20 -Jumps @(@(1000.0, -10.0), @(90000.0, -12.0)) -Unmatched @()
Check 'exactly 1 break per 10 minutes is inside the limit' ((Score (New-Compare) $twoIn20 (New-Play)).Verdict -eq 'PASS')
$short = New-RefAnalysis -Minutes 3 -Jumps @(,@(3877.0, -10.96)) -Unmatched @(,@(3877.0, 22.7))
$v = Score (New-Compare) $short (New-Play)
Check 'a single break in a 3-minute run is more than 1 per 10 minutes, so INVALID' ($v.Verdict -eq 'INVALID' -and ($v.InvalidReasons -join ' ') -match '3\.3 per 10 minutes')
Check 'a reference capture that is missing is INVALID' ((Score (New-Compare) $null (New-Play) $null).Verdict -eq 'INVALID')
Check 'an app recording that was not compared is INVALID' ((Score $null (New-RefAnalysis) (New-Play)).Verdict -eq 'INVALID')
$v = Score (New-Compare -Aligned $false) (New-RefAnalysis) (New-Play)
Check 'a recording that cannot be aligned with one fixed offset is INVALID, not FAIL' ($v.Verdict -eq 'INVALID' -and ($v.InvalidReasons -join ' ') -match 'misaligned')
$v = Score (New-Compare -AppOnly 5000) (New-RefAnalysis) (New-Play)
Check 'audio the app recorded that the reference lacks (incomplete reference) is INVALID' ($v.Verdict -eq 'INVALID' -and ($v.InvalidReasons -join ' ') -match 'incomplete reference')
$v = Score (New-Compare) (New-RefAnalysis) (New-Play) ([pscustomobject]@{ overrun_samples = 480; xruns = 0 })
Check 'a reference capture that lost samples itself is INVALID' ($v.Verdict -eq 'INVALID' -and ($v.InvalidReasons -join ' ') -match 'lost samples itself')
$noLate = [pscustomobject]@{ underrun_callbacks = 0 }
$v = Score (New-Compare) (New-RefAnalysis) $noLate
Check 'a player without late-callback data is INVALID, not PASS' ($v.Verdict -eq 'INVALID' -and ($v.InvalidReasons -join ' ') -match 'no late-callback data')
$v = Score (New-Compare -Differing 5) (New-RefAnalysis -Minutes 3 -Jumps @(,@(3877.0, -10.96))) (New-Play)
Check 'INVALID takes precedence over a fidelity FAIL (the run is repeated)' ($v.Verdict -eq 'INVALID' -and $v.FailReasons.Count -eq 1)
$text = (Get-WbVerdictLines -Result $v -Compare (New-Compare -Differing 5)) -join "`n"
Check 'the evidence says INVALID needs a repeat and still lists the fidelity result' ($text -match 'validity: INVALID \(no verdict; the run must be repeated\)' -and $text -match 'fidelity FAIL')

# ---- 5. The first, too sensitive late-callback data is judged by the buffer ---------------------------
$oldFormat = [pscustomobject]@{
  underrun_callbacks = 0; late_callbacks = 120; max_callback_interval_ms = 21.5
  late_callback_events = @([pscustomobject]@{ source_ms = 3880.0; at_ms = 5000.0; interval_ms = 20.0; nominal_ms = 10.0 })
}
$l = Get-WbPlayerLateCallbacks -Play $oldFormat
Check 'data from the 1.5-times threshold with a longest gap under the buffer holds no late callback' ($l.Known -and $l.Events.Count -eq 0)

if ($fails) { Write-Host "$fails failed"; exit 1 }
Write-Host 'Fidelity tests passed'
