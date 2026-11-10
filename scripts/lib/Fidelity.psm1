<#
  Fidelity.psm1: how the recorder gates judge a recording (rule dated 9 Nov 2026, docs/PLAN.md
  section 12, "Phase 1 recording-gate rule").

  The app is judged against what Windows delivered, the reference capture, not against the source
  signal: the virtual cable and the audio engine sit between the source and both captures and
  sometimes repeat a block (about 500 to 580 frames, one WASAPI period), which no recorder can
  prevent or see.

    a. Fidelity (decides PASS or FAIL): the app's recording equals the reference capture sample
       for sample over the whole run, after one fixed alignment offset per run of the app (found
       at its start, never adjusted later), and dropped_samples is 0. Any difference is a FAIL.
    b. Upstream breaks (reported, not gated): breaks between the source and the reference. Each is
       listed with its source position, wall-clock time, repeat and unmatched lengths, the nearest
       reference xrun and the player's late callbacks within 500 ms.
    c. Validity: the run is INVALID (no verdict, re-run needed) if any upstream break has a late
       player callback within 500 ms, or more than 250 ms unmatched, or if there is more than 1
       upstream break per 10 minutes of audio, or if the reference capture is missing or
       misaligned (or its data is incomplete: see Get-WbRunVerdict).
    d. The start-up xrun rule for the app's own xruns (Get-WbXrunVerdict) is unchanged.

  Amendment 1 (9 Nov 2026), after the first re-scoring showed two specification errors:
    a. Late-callback data is required only for a run that has an upstream break. A run with 0
       upstream breaks needs none. Reason: the data exists to attribute a break; with no break
       there is nothing to attribute.
    b. The break limit is a count per run, not a rate: at most max(1, floor(minutes of audio /
       10)) counted upstream breaks. Reason: a rate made a single break in a 3-minute run
       INVALID (3.3 per 10 minutes), which is not what the limit is for.
    c. Upstream breaks within the first 3 s of playback are reported but not counted toward the
       limit in (b). The 250 ms unmatched and the late-callback conditions still apply to them.
       Reason: consistent with the start-up allowance for the app's own xruns; the cable
       replays stale buffer audio at start-up (see the alignment note in docs/PLAN.md).
    The fidelity check (the app equals the reference after one alignment, 0 dropped) does not
    change.

  ASCII only; Windows PowerShell 5.1.
#>
Import-Module (Join-Path $PSScriptRoot 'GateRun.psm1')

# Limits of the validity rule (c).
$script:LateWindowMs = 500
$script:MaxUnmatchedMs = 250
# Amendment 1 (b): at most max(1, floor(minutes of audio / 10)) counted breaks per run.
$script:MinBreakLimit = 1
$script:MinutesPerBreak = 10
# Amendment 1 (c): breaks before this source position (the first 3 s of playback) are reported
# but not counted toward the limit.
$script:StartupMs = 3000
# Events closer than this are one break (a repeat and the unmatched stretch beside it).
$script:ClusterMs = 200
# The player's output buffer: only a gap longer than this can run it dry.
$script:PlayerBufferMs = 100

# Compares the app's recording with the reference capture (wb-testbench compare) and writes
# compare.json into the run folder. The start of the signal in each recording (from the source
# analyses, analysis.json and reference-analysis.json in the run folder) tells the tool where to
# take the alignment snippet: the cable can hold stale audio before the signal, which repeats
# itself and would match at the wrong offset. Returns the command result (ExitCode, Output, Error).
function Invoke-WbCompare {
  param(
    [Parameter(Mandatory = $true)] [string]$Testbench,
    [Parameter(Mandatory = $true)] [string]$SessionDir,
    [Parameter(Mandatory = $true)] [string]$ReferenceDir,
    [Parameter(Mandatory = $true)] [string]$RunDir,
    # The force-kill test stops the app before the player: the signal outlives the session.
    [switch]$IgnoreReferenceTail
  )
  $args2 = @('compare', '--app', $SessionDir, '--reference', $ReferenceDir, '--report', (Join-Path $RunDir 'compare.json'))
  foreach ($pair in @(@('analysis.json', '--app-signal-start-ms'), @('reference-analysis.json', '--reference-signal-start-ms'))) {
    $f = Join-Path $RunDir $pair[0]
    if (Test-Path -LiteralPath $f) {
      $j = Get-Content -LiteralPath $f -Raw | ConvertFrom-Json
      if (Test-WbHasProperty $j 'content_start_ms') { $args2 += @($pair[1], ([double]$j.content_start_ms).ToString([Globalization.CultureInfo]::InvariantCulture)) }
    }
  }
  if ($IgnoreReferenceTail) { $args2 += @('--reference-tail', 'ignore') }
  Invoke-WbNative -FilePath $Testbench -ArgumentList $args2
}

# The player's late render callbacks under the rule: gaps longer than its output buffer. Known is
# false when play.json has no callback data (a player before 9 Nov 2026 20:34). Data written with
# the first, too sensitive threshold (gaps of 1.5 times the audio before a callback) is re-judged:
# its longest gap is recorded, and a longest gap under the buffer means no late callback.
function Get-WbPlayerLateCallbacks {
  param($Play)
  if ($null -eq $Play -or -not (Test-WbHasProperty $Play 'max_callback_interval_ms')) {
    return [pscustomobject]@{ Known = $false; MaxGapMs = $null; Events = @() }
  }
  $max = [double]$Play.max_callback_interval_ms
  $events = @()
  if ($max -gt $script:PlayerBufferMs -and (Test-WbHasProperty $Play 'late_callback_events')) {
    $events = @(@($Play.late_callback_events) | Where-Object { $null -ne $_ -and [double]$_.interval_ms -gt $script:PlayerBufferMs })
  }
  [pscustomobject]@{ Known = $true; MaxGapMs = $max; Events = @($events) }
}

# Upstream breaks: what happened between the source and the reference capture. A repeat and the
# unmatched stretch beside it are one break.
#   PlayStarted: DateTimeOffset of the player's first callback (for wall-clock times), or $null.
function Get-WbUpstreamBreaks {
  param($RefAnalysis, $Play, $PlayStarted)
  if ($null -eq $RefAnalysis) { return @() }
  $items = @()
  foreach ($d in @($RefAnalysis.discontinuities)) {
    if ($null -eq $d) { continue }
    $items += [pscustomobject]@{ SourceMs = [double]$d.source_ms; Kind = 'jump'; Ms = [double]$d.source_jump_ms }
  }
  foreach ($u in @($RefAnalysis.unmatched_ranges)) {
    if ($null -eq $u) { continue }
    $items += [pscustomobject]@{ SourceMs = [double]$u.source_ms; Kind = 'unmatched'; Ms = [double]$u.ms }
  }
  $items = @($items | Sort-Object SourceMs)
  $late = Get-WbPlayerLateCallbacks -Play $Play
  $xruns = @()
  if (Test-WbHasProperty $RefAnalysis 'xruns') { $xruns = @($RefAnalysis.xruns) }
  $rate = $(if (Test-WbHasProperty $RefAnalysis 'sample_rate') { [double]$RefAnalysis.sample_rate } else { 48000.0 })
  $firstMs = [double]$RefAnalysis.content_first_source_frame * 1000.0 / $rate
  $breaks = @()
  $group = @()
  $groupEnd = $null
  $flush = {
    param($g)
    if ($g.Count -eq 0) { return }
    $src = ($g | Measure-Object SourceMs -Minimum).Minimum
    $unmatched = ($g | Where-Object { $_.Kind -eq 'unmatched' } | Measure-Object Ms -Sum).Sum
    $repeat = ($g | Where-Object { $_.Kind -eq 'jump' -and $_.Ms -lt 0 } | ForEach-Object { - $_.Ms } | Measure-Object -Sum).Sum
    $skipped = ($g | Where-Object { $_.Kind -eq 'jump' -and $_.Ms -gt 0 } | Measure-Object Ms -Sum).Sum
    $wall = $null
    if ($null -ne $PlayStarted) { $wall = ([DateTimeOffset]$PlayStarted).AddMilliseconds($src - $firstMs) }
    $nearest = $null
    if ($null -ne $wall -and $xruns.Count -gt 0) {
      $nearest = @($xruns | ForEach-Object {
          $w = [DateTimeOffset]::Parse($_.wall_clock)
          [pscustomobject]@{ N = $_.n; Wall = $w; DeltaMs = ($w - $wall).TotalMilliseconds }
        } | Sort-Object { [math]::Abs($_.DeltaMs) } | Select-Object -First 1)
    }
    $nearLate = @($late.Events | Where-Object { [math]::Abs([double]$_.source_ms - $src) -le $script:LateWindowMs })
    [pscustomobject]@{
      SourceMs      = [double]$src
      Wall          = $wall
      UnmatchedMs   = [double]$(if ($unmatched) { $unmatched } else { 0 })
      RepeatMs      = [double]$(if ($repeat) { $repeat } else { 0 })
      SkippedMs     = [double]$(if ($skipped) { $skipped } else { 0 })
      NearestXrun   = $(if ($nearest) { $nearest[0] } else { $null })
      LateCallbacks = @($nearLate)
    }
  }
  foreach ($it in $items) {
    if ($group.Count -gt 0 -and $it.SourceMs -gt $groupEnd + $script:ClusterMs) {
      $breaks += & $flush $group
      $group = @()
    }
    $group += $it
    $end = $it.SourceMs + $(if ($it.Kind -eq 'unmatched') { $it.Ms } else { 0 })
    $groupEnd = $(if ($null -eq $groupEnd -or $group.Count -eq 1) { $end } else { [math]::Max($groupEnd, $end) })
  }
  if ($group.Count -gt 0) { $breaks += & $flush $group }
  @($breaks | Where-Object { $null -ne $_ })
}

# The verdict of a recorder-gate run under the rule: PASS, FAIL or INVALID.
#   Compare:      compare.json (wb-testbench compare): the app's recording against the reference
#   RefAnalysis:  reference-analysis.json (the reference capture against the source)
#   Play, Reference: play.json and reference.json; DroppedSamples: the app's counter
function Get-WbRunVerdict {
  param(
    $Compare,
    $RefAnalysis,
    $Play,
    $Reference,
    $PlayStarted,
    [int64]$DroppedSamples = 0
  )
  $invalid = @()
  $fail = @()

  # --- validity (c)
  $referenceOk = ($null -ne $Reference -and $null -ne $RefAnalysis)
  if (-not $referenceOk) { $invalid += 'the reference capture is missing (no reference report or analysis)' }
  if ($referenceOk -and (Test-WbHasProperty $Reference 'overrun_samples') -and [int64]$Reference.overrun_samples -gt 0) {
    $invalid += "the reference capture lost samples itself (its ring overran by $($Reference.overrun_samples))"
  }
  if ($null -eq $Compare) {
    $invalid += 'the app was not compared with the reference capture (no comparison data)'
  } else {
    if (-not [bool]$Compare.aligned) { $invalid += 'the app and the reference capture could not be aligned with one fixed offset (misaligned)' }
    if ([int64]$Compare.app_only_loud_frames -gt 0) {
      $invalid += "the reference capture does not cover $($Compare.app_only_loud_frames) frames of audio the app recorded (incomplete reference)"
    }
  }
  $late = Get-WbPlayerLateCallbacks -Play $Play
  $breaks = @(Get-WbUpstreamBreaks -RefAnalysis $RefAnalysis -Play $Play -PlayStarted $PlayStarted)
  # Amendment 1 (a): late-callback data is needed only to attribute a break.
  if ($breaks.Count -gt 0 -and -not $late.Known) {
    $invalid += 'the run has an upstream break and the player recorded no late-callback data (a player before 9 Nov 2026 20:34), so the break cannot be attributed'
  }
  foreach ($b in $breaks) {
    if ($b.LateCallbacks.Count -gt 0) {
      $invalid += ("an upstream break at source {0:n3} s has {1} late player callback(s) within {2} ms (the player may have caused it)" -f ($b.SourceMs / 1000.0), $b.LateCallbacks.Count, $script:LateWindowMs)
    }
    if ($b.UnmatchedMs -gt $script:MaxUnmatchedMs) {
      $invalid += ("an upstream break at source {0:n3} s has {1:n0} ms unmatched (limit {2} ms)" -f ($b.SourceMs / 1000.0), $b.UnmatchedMs, $script:MaxUnmatchedMs)
    }
  }
  $minutes = 0.0
  if ($RefAnalysis -and (Test-WbHasProperty $RefAnalysis 'recorded_ms')) { $minutes = [double]$RefAnalysis.recorded_ms / 60000.0 }
  # Amendment 1 (b) and (c): a count per run, not counting breaks in the first 3 s of playback.
  $counted = @($breaks | Where-Object { $_.SourceMs -ge $script:StartupMs })
  $limit = [int][math]::Max($script:MinBreakLimit, [math]::Floor($minutes / $script:MinutesPerBreak))
  if ($counted.Count -gt $limit) {
    $invalid += ("{0} counted upstream break(s) in {1:n1} minutes of audio; the limit is {2} (max(1, floor(minutes / 10)); breaks in the first 3 s of playback are not counted)" -f $counted.Count, $minutes, $limit)
  }

  # --- fidelity (a)
  if ($null -ne $Compare -and [bool]$Compare.aligned) {
    if ([int64]$Compare.differing_frames -gt 0) {
      $fail += "the app's recording differs from the reference capture in $($Compare.differing_frames) frame(s)"
    }
    if ([int64]$Compare.reference_only_loud_frames -gt 0) {
      $fail += "the reference capture holds $($Compare.reference_only_loud_frames) frame(s) of audio the app has no recording of"
    }
  }
  if ($DroppedSamples -ne 0) { $fail += "the recorder dropped $DroppedSamples sample(s)" }

  $verdict = $(if ($invalid.Count -gt 0) { 'INVALID' } elseif ($fail.Count -gt 0) { 'FAIL' } else { 'PASS' })
  [pscustomobject]@{
    Verdict        = $verdict
    InvalidReasons = @($invalid)
    FailReasons    = @($fail)
    Breaks         = @($breaks)
    CountedBreaks  = @($counted)
    BreakLimit     = $limit
    AudioMinutes   = $minutes
    Late           = $late
  }
}

# Evidence lines: the fidelity result, the upstream breaks and the validity reasons.
function Get-WbVerdictLines {
  param($Result, $Compare)
  $lines = @()
  $lines += 'recording rule (9 Nov 2026): fidelity decides PASS or FAIL; upstream breaks are reported; a run with limits exceeded is INVALID (docs/PLAN.md section 12)'
  if ($null -ne $Compare) {
    $lines += ("fidelity: app compared with the reference capture over {0:n0} frames ({1:n1} s) in {2} run(s) of the app; differing frames {3}; reference audio the app has no recording of {4} frame(s); audio only the app has {5} frame(s); restart gaps {6} frame(s)" -f $Compare.compared_frames, ([double]$Compare.compared_frames / [double]$Compare.sample_rate), @($Compare.segments).Count, $Compare.differing_frames, $Compare.reference_only_loud_frames, $Compare.app_only_loud_frames, $Compare.restart_gap_frames)
    if ((Test-WbHasProperty $Compare 'reference_tail_ignored') -and [bool]$Compare.reference_tail_ignored) {
      $lines += ("  reference audio after the app's last frame: {0} frame(s), not judged (this gate stops the app before the player, by design)" -f $Compare.reference_tail_loud_frames)
    }
    foreach ($s in @($Compare.segments)) {
      $off = $(if ($null -ne $s.offset_frames) { "$($s.offset_frames) frames (found by a stretch of the $($s.aligned_by))" } else { 'not found' })
      $lines += ("  run {0}: app frames {1} to {2}, one fixed offset {3}; compared {4}, differing {5}, largest difference {6}" -f ($s.index + 1), $s.app_start_frame, $s.app_end_frame, $off, $s.compared_frames, $s.differing_frames, $s.max_abs_diff)
      foreach ($r in @($s.differing_ranges | Select-Object -First 5)) {
        $lines += ("    differs at reference frame {0} (app frame {1}) for {2} frame(s)" -f $r.reference_frame, $r.app_frame, $r.frames)
      }
    }
  } else {
    $lines += 'fidelity: not computed (no comparison data)'
  }
  $lines += ("upstream breaks (between the source and the reference capture; reported, not gated): {0} in {1:n1} minutes of audio, {2} counted toward the limit of {3} (breaks in the first 3 s of playback are not counted)" -f @($Result.Breaks).Count, $Result.AudioMinutes, @($Result.CountedBreaks).Count, $Result.BreakLimit)
  foreach ($b in @($Result.Breaks)) {
    $wall = $(if ($b.Wall) { ('{0:HH:mm:ss.fff} IST' -f $b.Wall) } else { 'wall clock unknown' })
    $xr = $(if ($b.NearestXrun) { ("nearest reference xrun #{0} at {1:HH:mm:ss.fff} ({2:n0} ms {3})" -f $b.NearestXrun.N, $b.NearestXrun.Wall, [math]::Abs($b.NearestXrun.DeltaMs), $(if ($b.NearestXrun.DeltaMs -lt 0) { 'before' } else { 'after' })) } else { 'no reference xrun logged' })
    $lines += ("  at source {0:n3} s, {1}: repeat {2:n1} ms, skipped {3:n1} ms, unmatched {4:n1} ms; {5}; late player callbacks within {6} ms: {7}" -f ($b.SourceMs / 1000.0), $wall, $b.RepeatMs, $b.SkippedMs, $b.UnmatchedMs, $xr, $script:LateWindowMs, $b.LateCallbacks.Count)
  }
  if ($Result.Late.Known) {
    $lines += ("player callbacks: longest gap {0:n1} ms against a {1} ms output buffer; late callbacks (a gap longer than the buffer) {2}" -f $Result.Late.MaxGapMs, $script:PlayerBufferMs, @($Result.Late.Events).Count)
  } else {
    $lines += $(if (@($Result.Breaks).Count -gt 0) { 'player callbacks: no late-callback data in play.json' } else { 'player callbacks: no late-callback data in play.json (not needed: the run has no upstream break)' })
  }
  if ($Result.InvalidReasons.Count -gt 0) {
    $lines += 'validity: INVALID (no verdict; the run must be repeated)'
    foreach ($r in $Result.InvalidReasons) { $lines += "  - $r" }
  } else {
    $lines += ("validity: valid (limits: a late player callback within {0} ms of a break, more than {1} ms unmatched in a break, more than max(1, floor(minutes / 10)) counted breaks, a missing, misaligned or incomplete reference, or an upstream break without late-callback data)" -f $script:LateWindowMs, $script:MaxUnmatchedMs)
  }
  if ($Result.FailReasons.Count -gt 0) {
    $lines += 'fidelity FAIL:'
    foreach ($r in $Result.FailReasons) { $lines += "  - $r" }
  }
  $lines
}

Export-ModuleMember -Function Invoke-WbCompare, Get-WbPlayerLateCallbacks, Get-WbUpstreamBreaks, Get-WbRunVerdict, Get-WbVerdictLines
