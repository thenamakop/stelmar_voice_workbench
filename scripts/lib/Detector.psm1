<#
  Detector.psm1: the live detector check of the recorder gates (Phase 2).

  The recorder marks two things it sees in the audio itself, in session.json: a repeated block
  (`repeated_block`) and a short run of exact digital silence inside live audio (`zero_gap`; what
  the shared upstream breaks of the 10 Nov 2026 runs are at the sample level). This module lines
  those events up with the shared upstream breaks the gate harness finds:

    * a shared break is MATCHED when an event lies within +-ToleranceMs of the break's span in the
      app's audio time. The span runs from the break's source position, mapped to the app's audio
      clock through the app analysis, to the end of its unmatched stretch (or its repeat/skip);
    * an event with no shared break is a FALSE ALARM;
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
      Matched = ($hits.Count -gt 0)
      Events = @($hits | ForEach-Object { $marks[$_] })
    }
  }
  $false_alarms = @()
  for ($i = 0; $i -lt $marks.Count; $i++) { if (-not $used.ContainsKey($i)) { $false_alarms += $marks[$i] } }
  [pscustomobject]@{
    ToleranceMs = $ToleranceMs
    Rows = $rows
    Matched = @($rows | Where-Object { $_.Matched }).Count
    Missed = @($rows | Where-Object { -not $_.Matched }).Count
    FalseAlarms = $false_alarms
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
  $lines += ("  shared upstream breaks found by the harness: {0}; matched {1}; missed {2}; events with no shared break (false alarms) {3}" -f @($Check.Rows).Count, $Check.Matched, $Check.Missed, @($Check.FalseAlarms).Count)
  foreach ($r in @($Check.Rows)) {
    $what = $(if ($r.Matched) { 'MATCHED by ' + ((@($r.Events) | ForEach-Object { "$($_.kind) at {0:n1} ms ($($_.detail))" -f [double]$_.audio_ms }) -join '; ') } else { 'MISSED: no repeated_block or zero_gap event in the span' })
    $lines += ("    break at source {0:n3} s, app audio {1:n1} to {2:n1} ms: {3}" -f ($r.SourceMs / 1000.0), $r.AppStartMs, $r.AppEndMs, $what)
  }
  foreach ($e in @($Check.FalseAlarms)) {
    $lines += ("    FALSE ALARM: {0} at {1:n1} ms ({2})" -f $e.kind, [double]$e.audio_ms, $e.detail)
  }
  if ($null -ne $Rec -and (Test-DetHas $Rec 'timestamp_outliers')) {
    $lines += ("  app counters after the PositionCheck fix: position breaks {0} (device time left the predicted line for 3 callbacks in a row), timestamp outliers {1} (came back; no audio affected), old pairwise flags {2} (what the rule before the fix counted: each outlier twice); repeated_block events {3}; zero_gap events {4}" -f $Rec.position_breaks, $Rec.timestamp_outliers, $Rec.position_pair_flags, $Rec.repeated_blocks, $Rec.zero_gaps)
  } else {
    $lines += '  app counters after the PositionCheck fix: not available (the report is from a build before Phase 2)'
  }
  $lines
}

Export-ModuleMember -Function Get-WbAppOffsetMs, Get-WbDetectorMatches, Get-WbDetectorLines
