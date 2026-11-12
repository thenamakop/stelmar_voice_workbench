<#
  Detector.psm1: the live detector check of the recorder gates (Phase 2).

  The recorder marks two things it sees in the audio itself, in session.json: a repeated block
  (`repeated_block`) and a short run of exact digital silence inside live audio (`zero_gap`; what
  the shared upstream breaks of the 10 Nov 2026 runs are at the sample level). This module lines
  those events up with the shared upstream breaks the gate harness finds:

    * a shared break is MATCHED when an event lies within +-ToleranceMs of the break's span in the
      app's audio time. The span runs from the break's source position, mapped to the app's audio
      clock through the app analysis, to the end of its unmatched stretch (or its repeat/skip);
    * an event with no shared break is a FALSE ALARM, unless it lies before the first source
      sample that reached the recording (the analysis's content_start_ms, on the app's audio clock):
      then it is a LEAD-IN event, the cable's stale audio or start-up before the test signal begins,
      where the source is not what the recorder was checked against. A lead-in event is reported,
      not counted as a false alarm (11 Nov 2026: the soak's zero_gap at 2,457.0 ms, 90.6 ms before
      the first source sample at 2,547.6 ms);
    * a shared break with no event is a MISS.

  Not gated in Phase 2: the evidence reports every miss and false alarm, and the run sequence stops
  for a decision before the speed tables. ASCII only; Windows PowerShell 5.1.
#>

function Test-DetHas {
  param($Obj, [string]$Name)
  if ($null -eq $Obj) { return $false }
  foreach ($p in $Obj.PSObject.Properties) { if ($p.Name -eq $Name) { return $true } }
  $false
}

# The offset (app audio ms minus source ms) in force just before the source position $SourceMs:
# the offset at the start of the signal, less the jump of every earlier discontinuity (a positive
# source jump means the source moved ahead of the recording, as it does across a restart).
function Get-WbAppOffsetMs {
  param($Analysis, [double]$SourceMs)
  $rate = $(if (Test-DetHas $Analysis 'sample_rate') { [double]$Analysis.sample_rate } else { 48000.0 })
  $first = $(if (Test-DetHas $Analysis 'content_first_source_frame') { [double]$Analysis.content_first_source_frame } else { 0.0 })
  $offset = [double]$Analysis.content_start_ms - $first * 1000.0 / $rate
  foreach ($d in @($Analysis.discontinuities)) {
    if ($null -eq $d) { continue }
    if ([double]$d.source_ms -lt $SourceMs - 1.0) { $offset -= [double]$d.source_jump_ms }
  }
  $offset
}

# The breaks the app's own analysis (the recording against the source) found, as spans of the app's
# audio time: each discontinuity at its position and each unmatched stretch with its length.
function Get-WbAppAnalysisSpans {
  param($Analysis)
  $spans = @()
  foreach ($d in @($Analysis.discontinuities)) {
    if ($null -eq $d) { continue }
    $spans += [pscustomobject]@{ Kind = 'discontinuity'; StartMs = [double]$d.at_ms; EndMs = [double]$d.at_ms }
  }
  foreach ($u in @($Analysis.unmatched_ranges)) {
    if ($null -eq $u) { continue }
    $start = [double]$u.source_ms + (Get-WbAppOffsetMs -Analysis $Analysis -SourceMs ([double]$u.source_ms))
    $spans += [pscustomobject]@{ Kind = 'unmatched'; StartMs = $start; EndMs = $start + [double]$u.ms }
  }
  $spans
}

# Lines up the harness's shared breaks with the recorder's events.
#   Breaks:   output of Get-WbUpstreamBreaks (SourceMs, UnmatchedMs, RepeatMs, SkippedMs)
#   Analysis: analysis.json (the app's recording against the source)
#   Events:   the events of session.json (audio_ms, kind, detail)
function Get-WbDetectorMatches {
  param($Breaks, $Analysis, $Events, [double]$ToleranceMs = 50.0)
  $marks = @(@($Events) | Where-Object { $null -ne $_ -and ($_.kind -eq 'repeated_block' -or $_.kind -eq 'zero_gap') })
  $used = @{}
  $rows = @()
  foreach ($b in @($Breaks)) {
    if ($null -eq $b) { continue }
    $off = Get-WbAppOffsetMs -Analysis $Analysis -SourceMs ([double]$b.SourceMs)
    $start = [double]$b.SourceMs + $off
    $len = [math]::Max([double]$b.UnmatchedMs, [math]::Max([double]$b.RepeatMs, [double]$b.SkippedMs))
    $end = $start + $len
    $hits = @()
    for ($i = 0; $i -lt $marks.Count; $i++) {
      $t = [double]$marks[$i].audio_ms
      if ($t -ge $start - $ToleranceMs -and $t -le $end + $ToleranceMs) { $hits += $i }
    }
    foreach ($i in $hits) { $used[$i] = $true }
    $rows += [pscustomobject]@{
      SourceMs = [double]$b.SourceMs
      AppStartMs = $start
      AppEndMs = $end
      UnmatchedMs = [double]$b.UnmatchedMs
      RepeatMs = [double]$b.RepeatMs
      # The break happened in the source before the app began recording: no recorder could see it.
      BeforeRecording = ($end + $ToleranceMs -lt 0)
      Matched = ($hits.Count -gt 0)
      Events = @($hits | ForEach-Object { $marks[$_] })
    }
  }
  $appSpans = @(Get-WbAppAnalysisSpans -Analysis $Analysis)
  # Where the first source sample that reached the recording falls on the app's audio clock
  # (content_start_ms; the soak's was 2,547.6 ms, 89.3 ms of the source's start never arrived). A run
  # that began recording part way into the source (the kill run) has 0 here: no event is a lead-in.
  $leadInEnd = $null
  if (Test-DetHas $Analysis 'content_start_ms') { $leadInEnd = [double]$Analysis.content_start_ms }
  $false_alarms = @()
  $lead_in = @()
  for ($i = 0; $i -lt $marks.Count; $i++) {
    if ($used.ContainsKey($i)) { continue }
    $t = [double]$marks[$i].audio_ms
    if ($null -ne $leadInEnd -and $t -lt $leadInEnd) {
      $lead_in += [pscustomobject]@{ kind = $marks[$i].kind; audio_ms = $t; detail = $marks[$i].detail; BeforeSourceMs = ($leadInEnd - $t) }
      continue
    }
    $near = @($appSpans | Where-Object { $t -ge $_.StartMs - $ToleranceMs -and $t -le $_.EndMs + $ToleranceMs })
    $false_alarms += [pscustomobject]@{
      kind = $marks[$i].kind
      audio_ms = $t
      detail = $marks[$i].detail
      # The app's own analysis (recording against source) has a break there too: the event is real,
      # the harness just did not count it as shared with the reference capture.
      InAppAnalysis = ($near.Count -gt 0)
    }
  }
  [pscustomobject]@{
    ToleranceMs = $ToleranceMs
    Rows = $rows
    Matched = @($rows | Where-Object { $_.Matched }).Count
    Missed = @($rows | Where-Object { -not $_.Matched -and -not $_.BeforeRecording }).Count
    BeforeRecording = @($rows | Where-Object { $_.BeforeRecording -and -not $_.Matched }).Count
    FalseAlarms = $false_alarms
    LeadIn = $lead_in
    LeadInEndMs = $leadInEnd
    EventCount = $marks.Count
  }
}

# Evidence lines for a gate run.
#   Rec: the recorder's counters at Stop (position_breaks, timestamp_outliers, position_pair_flags,
#        repeated_blocks, zero_gaps), or $null for an older build.
function Get-WbDetectorLines {
  param($Check, $Rec)
  $lines = @()
  $lines += 'live detector check (not gated in Phase 2; any miss or false alarm stops the run sequence for a decision before the speed tables):'
  $lines += ("  rule: a shared upstream break is matched when a repeated_block or zero_gap event of the app's session.json lies within +-{0:n0} ms of the break's span in the app's audio time (the span runs from the break's source position, mapped through the app analysis, to the end of its unmatched stretch); an event with no shared break is a false alarm" -f $Check.ToleranceMs)
  $lines += ("  shared upstream breaks found by the harness: {0}; matched {1}; missed {2}; before the app began recording {3}; events with no shared break (false alarms) {4}; lead-in events (before the first source sample that reached the recording) {5}" -f @($Check.Rows).Count, $Check.Matched, $Check.Missed, $Check.BeforeRecording, @($Check.FalseAlarms).Count, @($Check.LeadIn).Count)
  foreach ($r in @($Check.Rows)) {
    $what = $(if ($r.Matched) { 'MATCHED by ' + ((@($r.Events) | ForEach-Object { "$($_.kind) at {0:n1} ms ($($_.detail))" -f [double]$_.audio_ms }) -join '; ') } elseif ($r.BeforeRecording) { 'BEFORE THE APP BEGAN RECORDING: the break is in the source before the first recorded sample, so no recorder could see it (not counted as a miss)' } else { ('MISSED: no repeated_block or zero_gap event in the span (unmatched {0:n0} ms, repeat {1:n1} ms)' -f $r.UnmatchedMs, $r.RepeatMs) })
    $lines += ("    break at source {0:n3} s, app audio {1:n1} to {2:n1} ms: {3}" -f ($r.SourceMs / 1000.0), $r.AppStartMs, $r.AppEndMs, $what)
  }
  foreach ($e in @($Check.FalseAlarms)) {
    $why = $(if ((Test-DetHas $e 'InAppAnalysis') -and $e.InAppAnalysis) { '; the app-vs-source analysis has a break at this place too, so the event is real but the harness did not count it as shared with the reference capture' } else { '' })
    $lines += ("    FALSE ALARM: {0} at {1:n1} ms ({2}){3}" -f $e.kind, [double]$e.audio_ms, $e.detail, $why)
  }
  foreach ($e in @($Check.LeadIn)) {
    $lines += ("    LEAD-IN: {0} at {1:n1} ms ({2}), {3:n1} ms before the first source sample that reached the recording (app audio {4:n1} ms); the cable's start-up before the test signal, not a false alarm" -f $e.kind, [double]$e.audio_ms, $e.detail, [double]$e.BeforeSourceMs, [double]$Check.LeadInEndMs)
  }
  if ($null -ne $Rec -and (Test-DetHas $Rec 'timestamp_outliers')) {
    $lines += ("  app counters after the PositionCheck fix: position breaks {0} (device time left the predicted line for 3 callbacks in a row), timestamp outliers {1} (came back; no audio affected), old pairwise flags {2} (what the rule before the fix counted: each outlier twice); repeated_block events {3}; zero_gap events {4}" -f $Rec.position_breaks, $Rec.timestamp_outliers, $Rec.position_pair_flags, $Rec.repeated_blocks, $Rec.zero_gaps)
  } else {
    $lines += '  app counters after the PositionCheck fix: not available (the report is from a build before Phase 2)'
  }
  $lines
}

# Saves a run's detector check next to the run (detector-check.json) for detector-table.ps1.
function Save-WbDetectorCheck {
  param([string]$RunDir, [string]$Gate, [string]$Profile, $Check, $Rec, [double]$AudioMs, [string]$Verdict = '')
  $has = { param($o, $n) $null -ne $o -and $null -ne $o.PSObject.Properties[$n] }
  $counters = [ordered]@{}
  foreach ($n in 'repeated_blocks', 'zero_gaps', 'detector_max_block_us', 'detector_over_budget_blocks', 'detector_skipped_ms') {
    $counters[$n] = $(if (& $has $Rec $n) { $Rec.$n } else { $null })
  }
  $counters['detectors_disabled'] = $(if (& $has $Rec 'detectors_disabled') { @($Rec.detectors_disabled).Count } else { $null })
  $doc = [pscustomobject]@{
    gate = $Gate
    profile = $Profile
    verdict = $Verdict
    audio_ms = $AudioMs
    run = (Split-Path -Leaf $RunDir)
    audio_minutes = [math]::Round($AudioMs / 60000.0, 1)
    breaks = @($Check.Rows).Count
    matched = $Check.Matched
    missed = $Check.Missed
    before_recording = $Check.BeforeRecording
    false_alarms = @($Check.FalseAlarms).Count
    lead_in = @($Check.LeadIn).Count
    events = $Check.EventCount
    counters = $counters
  }
  ($doc | ConvertTo-Json -Depth 5) | Set-Content -LiteralPath (Join-Path $RunDir 'detector-check.json') -Encoding utf8
}

Export-ModuleMember -Function Get-WbAppOffsetMs, Get-WbAppAnalysisSpans, Get-WbDetectorMatches, Get-WbDetectorLines, Save-WbDetectorCheck
