<#
.SYNOPSIS
  The queue-soak gate as pure functions: its settings, the capability file it needs, the day plan, the checks on the
  transcript of session 1, the overlap of decoding with the second recording, and the verdict.
.DESCRIPTION
  The gate records two sessions through the cable with the app's real queue running (gate build, WB_GATE_DAY_PLAN):
  session 1 is public speech (FLEURS English and Hindi, CC BY 4.0) that the queue transcribes while session 2, seeded
  noise that the recorder analyser can judge sample for sample, is recorded. Nothing here starts a process or reads
  the machine; scripts\gates\queue-soak.ps1 does that and hands the results to these functions, so the verdict rules
  are tested with fake inputs (scripts\tests\QueueSoak.Tests.ps1).

  The verdict (owner, 23 Nov 2026):
    - the Phase 1 recording rule for BOTH sessions: the app equals both independent references sample for sample (one
      offset per run), 0 samples dropped, at most 9 counted upstream breaks per 90 minutes (session 2, whose source the
      analyser knows; session 1 is speech and is judged by equality alone), the recording as long as the references;
    - 0 ring-buffer overruns and 0 xruns (the Phase 1 excuse for one xrun at the start of the stream, with no loss
      around it, applies as in every other recorder gate);
    - session 1's segments committed in full: the job is done, its cursor is at the end of the audio, no overlap and no
      duplicate start, no gap longer than the limit, the first and last segment near the ends of the audio;
    - the "ready by" estimate error and the engine memory peak are information only.
  A run in which the queue did not decode session 1 while session 2 recorded measured nothing and is INVALID.

  ASCII only; Windows PowerShell 5.1.
#>
Set-StrictMode -Version 2

# The longest silence between two committed segments that is not a hole: read speech with 2 s pauses (the looped
# source) leaves gaps of a few seconds; a lost chunk of 30 s leaves one of 20 s or more.
$script:GapLimitMs = 20000
# Decoding of session 1 that must overlap the recording of session 2 for the run to have measured the queue under a
# recording at all.
$script:MinOverlapSeconds = 60

function Get-WbQueueSoakGapLimitMs { $script:GapLimitMs }
function Get-WbQueueSoakMinOverlapSeconds { $script:MinOverlapSeconds }

# Lengths of the two sessions and the time the queue gets to finish session 1 after the last recording. Quick mode is
# for a short test run (about 25 minutes on a laptop); full mode for the laptop run (about an hour).
function Get-WbQueueSoakSettings {
  param([switch]$Quick)
  if ($Quick) {
    return [pscustomobject]@{ Mode = 'quick'; Session1Seconds = 300; Session2Seconds = 300; PartSeconds = 120; DecodeWaitMinutes = 12; MaxMinutes = 45 }
  }
  # PartSeconds 0: the app's own part length (10 minutes).
  [pscustomobject]@{ Mode = 'full'; Session1Seconds = 1500; Session2Seconds = 1200; PartSeconds = 0; DecodeWaitMinutes = 30; MaxMinutes = 120 }
}

# The capability.json this pack run's own quick check kept, or why there is none. A missing, empty or unreadable file
# is the gate's PENDING (never FAIL): without a system check the app does not create a job.
#   Found: true when the file can be used; ProcessWhileRecording: what the check decided (null when unknown)
function Get-WbQueueSoakCapability {
  param([Parameter(Mandatory = $true)] [string]$Path)
  $none = { param($why) [pscustomobject]@{ Found = $false; Path = $Path; Reason = $why; ProcessWhileRecording = $null } }
  if (-not (Test-Path -LiteralPath $Path)) {
    return (& $none 'the quick check of this pack run has not produced a capability.json (it has not run on this laptop yet, or it kept none): run it first (Start-Gates.cmd -Only quick-check), then this gate')
  }
  $text = ''
  try { $text = [IO.File]::ReadAllText($Path, [Text.Encoding]::UTF8) } catch { return (& $none "the capability.json of the quick check cannot be read: $($_.Exception.Message)") }
  if ([string]::IsNullOrWhiteSpace($text)) { return (& $none 'the capability.json of the quick check is empty') }
  $j = $null
  try { $j = $text | ConvertFrom-Json } catch { return (& $none "the capability.json of the quick check is not valid JSON: $($_.Exception.Message)") }
  if ($null -eq $j -or -not $j.PSObject.Properties['decision']) { return (& $none 'the capability.json of the quick check has no decision section') }
  $pwr = $null
  if ($j.decision.PSObject.Properties['process_while_recording']) { $pwr = [bool]$j.decision.process_while_recording }
  [pscustomobject]@{ Found = $true; Path = $Path; Reason = ''; ProcessWhileRecording = $pwr }
}

# The titles of the two sessions (their folders are named after them by the app).
function Get-WbQueueSoakTitles { [pscustomobject]@{ Speech = 'Queue soak speech'; Noise = 'Queue soak noise' } }

# The day plan of the two sessions (the app's WB_GATE_DAY_PLAN file, as an object for ConvertTo-Json): session 1 at
# once, session 2 after it, each stopped by its own stop file. New-WbDayPlanSessions (lib\DaySim.psm1) does the timing.
function New-WbQueueSoakPlan {
  param(
    [Parameter(Mandatory = $true)] [int]$Session1Seconds,
    [Parameter(Mandatory = $true)] [int]$Session2Seconds,
    [Parameter(Mandatory = $true)] [string]$RunDir,
    [Parameter(Mandatory = $true)] [string]$Device,
    [string[]]$Languages = @('en', 'hi')
  )
  $t = Get-WbQueueSoakTitles
  $sessions = @(
    [pscustomobject]@{ title = $t.Speech; seconds = $Session1Seconds; languages = $Languages },
    [pscustomobject]@{ title = $t.Noise; seconds = $Session2Seconds; languages = $Languages }
  )
  $planned = @(New-WbDayPlanSessions -Sessions $sessions -RunDir $RunDir -Device $Device -Programme $false)
  [ordered]@{ sessions = $planned; actions = @() }
}

# ---- the transcript of session 1 -------------------------------------------------------------------------------------

# Checks on the committed segments of one revision (post-1.jsonl, as objects with start_ms and end_ms): no overlap
# (the committed order is the audio order), no duplicate start, no gap longer than the limit, the first and the last
# segment near the ends of the audio. Gaps are judged only where the audio is speech: the caller says so.
function Test-WbQueueSoakSegments {
  param(
    [AllowEmptyCollection()] [AllowNull()] [object[]]$Segments,
    [Parameter(Mandatory = $true)] [double]$AudioMs,
    [int]$GapLimitMs = $script:GapLimitMs
  )
  $segs = @(@($Segments) | Where-Object { $null -ne $_ } | Sort-Object { [double]$_.start_ms })
  $problems = New-Object System.Collections.Generic.List[string]
  $overlaps = 0; $dups = 0; $maxGap = 0.0; $maxGapAt = 0.0
  if ($segs.Count -eq 0) {
    $problems.Add('no segment was committed')
    return [pscustomobject]@{ Count = 0; Overlaps = 0; DuplicateStarts = 0; MaxGapMs = 0.0; MaxGapAtMs = 0.0; FirstStartMs = $null; LastEndMs = $null; AudioMs = $AudioMs; Ok = $false; Problems = @($problems.ToArray()) }
  }
  for ($i = 1; $i -lt $segs.Count; $i++) {
    $a = $segs[$i - 1]; $b = $segs[$i]
    if ([double]$b.start_ms -eq [double]$a.start_ms) { $dups++ }
    if ([double]$b.start_ms -lt [double]$a.end_ms - 1) { $overlaps++ }
    $gap = [double]$b.start_ms - [double]$a.end_ms
    if ($gap -gt $maxGap) { $maxGap = $gap; $maxGapAt = [double]$a.end_ms }
  }
  $first = [double]$segs[0].start_ms
  $last = [double]($segs | ForEach-Object { [double]$_.end_ms } | Measure-Object -Maximum).Maximum
  if ($overlaps -gt 0) { $problems.Add("$overlaps segment(s) overlap the one before them") }
  if ($dups -gt 0) { $problems.Add("$dups segment(s) start at the same time as another (a duplicate)") }
  if ($maxGap -gt $GapLimitMs) { $problems.Add(('a gap of {0:n1} s after {1:n1} s with no segment (limit {2:n0} s)' -f ($maxGap / 1000.0), ($maxGapAt / 1000.0), ($GapLimitMs / 1000.0))) }
  if ($first -gt $GapLimitMs) { $problems.Add(('the first segment starts at {0:n1} s (limit {1:n0} s)' -f ($first / 1000.0), ($GapLimitMs / 1000.0))) }
  if ($AudioMs -gt 0 -and $last -lt $AudioMs - $GapLimitMs) { $problems.Add(('the last segment ends at {0:n1} s of {1:n1} s of audio (the end of the audio is missing)' -f ($last / 1000.0), ($AudioMs / 1000.0))) }
  if ($AudioMs -gt 0 -and $last -gt $AudioMs + 2000) { $problems.Add(('the last segment ends at {0:n1} s, after the audio ({1:n1} s)' -f ($last / 1000.0), ($AudioMs / 1000.0))) }
  [pscustomobject]@{
    Count = $segs.Count; Overlaps = $overlaps; DuplicateStarts = $dups; MaxGapMs = $maxGap; MaxGapAtMs = $maxGapAt
    FirstStartMs = $first; LastEndMs = $last; AudioMs = $AudioMs; Ok = ($problems.Count -eq 0); Problems = @($problems.ToArray())
  }
}

# The job of a session and revision in the queue report (report.json: queue.snapshot.jobs). $Slug is the folder name of
# the session. Null when the report has no such job.
function Get-WbQueueSoakJob {
  param($Report, [Parameter(Mandatory = $true)] [string]$Slug, [string]$Revision = 'post-1')
  if ($null -eq $Report -or -not $Report.PSObject.Properties['queue']) { return $null }
  $q = $Report.queue
  if ($null -eq $q -or -not $q.PSObject.Properties['snapshot'] -or $null -eq $q.snapshot) { return $null }
  if (-not $q.snapshot.PSObject.Properties['jobs']) { return $null }
  @(@($q.snapshot.jobs) | Where-Object { $_.session -eq $Slug -and $_.revision -eq $Revision }) | Select-Object -First 1
}

# Is the job finished in full: state done, the cursor at the end of the audio, every unit committed, no error.
function Test-WbQueueSoakJobComplete {
  param($Job)
  if ($null -eq $Job) { return [pscustomobject]@{ Ok = $false; Reason = 'the queue has no transcription job for session 1' } }
  $err = ''
  if ($Job.PSObject.Properties['error'] -and $null -ne $Job.error -and "$($Job.error)" -ne '') { $err = "$($Job.error)" }
  if ($err) { return [pscustomobject]@{ Ok = $false; Reason = "the job of session 1 reports an error: $err" } }
  if ("$($Job.state)" -ne 'done') { return [pscustomobject]@{ Ok = $false; Reason = "the job of session 1 is '$($Job.state)', not done (cursor $($Job.cursor_ms) of $($Job.audio_ms) ms, units $($Job.units_committed) of $($Job.units_total))" } }
  if ([double]$Job.cursor_ms -lt [double]$Job.audio_ms) { return [pscustomobject]@{ Ok = $false; Reason = "the job of session 1 is done but its cursor is at $($Job.cursor_ms) ms of $($Job.audio_ms) ms" } }
  if ([int]$Job.units_committed -ne [int]$Job.units_total) { return [pscustomobject]@{ Ok = $false; Reason = "the job of session 1 is done with $($Job.units_committed) of $($Job.units_total) units committed" } }
  [pscustomobject]@{ Ok = $true; Reason = '' }
}

# ---- decoding while session 2 records --------------------------------------------------------------------------------

# Seconds in which at least one unit of session 1 was being decoded inside the window of the second recording. Each
# unit is [At - Seconds, At] ("unit N of post-1 done in X s" read at wall time At); units that ran at once (the hybrid
# CPU and GPU pair) are counted once.
function Get-WbDecodeOverlapSeconds {
  param(
    [AllowEmptyCollection()] [AllowNull()] [object[]]$Units,
    [Parameter(Mandatory = $true)] [datetime]$WindowStart,
    [Parameter(Mandatory = $true)] [datetime]$WindowEnd
  )
  $iv = @()
  foreach ($u in @($Units)) {
    if ($null -eq $u) { continue }
    $s = ([datetime]$u.At).AddSeconds(-[double]$u.Seconds)
    $e = [datetime]$u.At
    if ($s -lt $WindowStart) { $s = $WindowStart }
    if ($e -gt $WindowEnd) { $e = $WindowEnd }
    if ($e -gt $s) { $iv += [pscustomobject]@{ S = $s; E = $e } }
  }
  if ($iv.Count -eq 0) { return 0.0 }
  $iv = @($iv | Sort-Object { $_.S })
  $total = 0.0
  $curS = $iv[0].S; $curE = $iv[0].E
  for ($i = 1; $i -lt $iv.Count; $i++) {
    if ($iv[$i].S -le $curE) { if ($iv[$i].E -gt $curE) { $curE = $iv[$i].E } }
    else { $total += ($curE - $curS).TotalSeconds; $curS = $iv[$i].S; $curE = $iv[$i].E }
  }
  $total + ($curE - $curS).TotalSeconds
}

# ---- the queue log ----------------------------------------------------------------------------------------------------

# "unit N of post-1 done in X s on <device>" lines of one session: Unit, Seconds, Device (the wall time is added by the
# caller, which knows when it read the line).
function ConvertFrom-WbQueueUnitLine {
  param([string]$Line, [Parameter(Mandatory = $true)] [string]$Slug)
  $m = [regex]::Match($Line, ('(?:^|\t)' + [regex]::Escape($Slug) + ': unit (\d+) of post-1 done in ([0-9.]+) s on (\w+)'))
  if (-not $m.Success) { return $null }
  [pscustomobject]@{ Unit = [int]$m.Groups[1].Value; Seconds = [double]::Parse($m.Groups[2].Value, [Globalization.CultureInfo]::InvariantCulture); Device = $m.Groups[3].Value }
}

function ConvertTo-WbClockMinutes {
  param([string]$Clock)
  $m = [regex]::Match($Clock, '^(\d{1,2}):(\d{2})')
  if (-not $m.Success) { return $null }
  [int]$m.Groups[1].Value * 60 + [int]$m.Groups[2].Value
}

# Information from the queue log (lines of queue.log: "<clock>\t<text>"): the engine memory peaks, and the "ready by"
# estimate the queue gave when session 1 stopped against the clock time of its completion. Nothing here is a bar.
function Get-WbQueueSoakLogInfo {
  param([AllowEmptyCollection()] [AllowNull()] [string[]]$Lines, [Parameter(Mandatory = $true)] [string]$Slug)
  $peaks = @(); $limits = @()
  $estimateClock = $null; $completeClock = $null
  foreach ($l in @($Lines)) {
    if ($null -eq $l) { continue }
    $p = [regex]::Match($l, 'engine peak (\d+) MiB against limit (\d+) MiB')
    if ($p.Success -and $l -match [regex]::Escape($Slug)) { $peaks += [int]$p.Groups[1].Value; $limits += [int]$p.Groups[2].Value }
    $e = [regex]::Match($l, ('^(\d{1,2}:\d{2})\s+' + [regex]::Escape($Slug) + ': stopped\. All transcripts ready by (\d{1,2}:\d{2}) IST'))
    if ($e.Success -and $null -eq $estimateClock) { $estimateClock = [pscustomobject]@{ At = $e.Groups[1].Value; ReadyBy = $e.Groups[2].Value } }
    $c = [regex]::Match($l, ('^(\d{1,2}:\d{2})\s+' + [regex]::Escape($Slug) + ': post-1 transcript complete'))
    if ($c.Success) { $completeClock = $c.Groups[1].Value }
  }
  $errMin = $null
  if ($estimateClock -and $completeClock) {
    $est = ConvertTo-WbClockMinutes $estimateClock.ReadyBy
    $act = ConvertTo-WbClockMinutes $completeClock
    if ($null -ne $est -and $null -ne $act) {
      $d = $act - $est
      if ($d -gt 720) { $d -= 1440 } elseif ($d -lt -720) { $d += 1440 }
      $errMin = $d
    }
  }
  [pscustomobject]@{
    EnginePeakMiB = $(if ($peaks.Count) { ($peaks | Measure-Object -Maximum).Maximum } else { $null })
    EngineLimitMiB = $(if ($limits.Count) { ($limits | Measure-Object -Minimum).Minimum } else { $null })
    EstimateGivenAt = $(if ($estimateClock) { $estimateClock.At } else { $null })
    EstimatedReadyBy = $(if ($estimateClock) { $estimateClock.ReadyBy } else { $null })
    CompletedAt = $completeClock
    EstimateErrorMinutes = $errMin
  }
}

# ---- the verdict ------------------------------------------------------------------------------------------------------

# One session's recording under the Phase 1 rule.
#   RunVerdict:  Get-WbRunVerdict of the app against the cpal reference (a session whose source the analyser knows);
#                $null for the speech session
#   RefVerdict:  Get-PackRefVerdict of the app against both references
#   Dropped, Overrun: the recorder's counters; Xruns: its count, XrunExcused: true when Get-WbXrunVerdict excused them
#   ReferenceOverrun: samples the reference capture lost itself (makes the session INVALID)
function Get-WbQueueSoakRecordingVerdict {
  param(
    [Parameter(Mandatory = $true)] [string]$Label,
    $RunVerdict = $null,
    $RefVerdict = $null,
    [int64]$Dropped = 0,
    [int64]$Overrun = 0,
    [int64]$Xruns = 0,
    [bool]$XrunExcused = $false,
    [int64]$ReferenceOverrun = 0
  )
  $fail = New-Object System.Collections.Generic.List[string]
  $invalid = New-Object System.Collections.Generic.List[string]
  if ($null -eq $RefVerdict) { $invalid.Add("$Label`: the app was not compared with both references") }
  elseif ($RefVerdict.Verdict -eq 'FAIL') { $fail.Add("$Label`: $($RefVerdict.Reason)") }
  elseif ($RefVerdict.Verdict -eq 'INVALID') { $invalid.Add("$Label`: $($RefVerdict.Reason)") }
  if ($null -ne $RunVerdict) {
    if ($RunVerdict.Verdict -eq 'FAIL') { foreach ($r in @($RunVerdict.FailReasons)) { $fail.Add("$Label`: $r") }; if (@($RunVerdict.FailReasons).Count -eq 0) { $fail.Add("$Label`: the Phase 1 recording rule failed") } }
    elseif ($RunVerdict.Verdict -eq 'INVALID') { foreach ($r in @($RunVerdict.InvalidReasons)) { $invalid.Add("$Label`: $r") }; if (@($RunVerdict.InvalidReasons).Count -eq 0) { $invalid.Add("$Label`: the Phase 1 recording rule found the run invalid") } }
  }
  if ($Dropped -gt 0) { $fail.Add("$Label`: $Dropped sample(s) dropped by the writer") }
  if ($Overrun -gt 0) { $fail.Add("$Label`: $Overrun sample(s) lost to ring-buffer overruns") }
  if ($Xruns -gt 0 -and -not $XrunExcused) { $fail.Add("$Label`: $Xruns driver xrun(s) (not excused)") }
  if ($ReferenceOverrun -gt 0) { $invalid.Add("$Label`: the reference capture lost $ReferenceOverrun sample(s) itself") }
  $status = $(if ($fail.Count -gt 0) { 'FAIL' } elseif ($invalid.Count -gt 0) { 'INVALID' } else { 'PASS' })
  [pscustomobject]@{ Label = $Label; Status = $status; Fail = @($fail.ToArray()); Invalid = @($invalid.ToArray()) }
}

# The verdict of the gate. A failure of a check always counts; an INVALID (a disturbed rig, a run in which the queue did
# no work during the second recording) only decides when nothing failed.
#   Recording1, Recording2: Get-WbQueueSoakRecordingVerdict results
#   Segments: Test-WbQueueSoakSegments result; JobCheck: Test-WbQueueSoakJobComplete result
#   OverlapSeconds: decoding of session 1 inside the recording of session 2
function Get-WbQueueSoakVerdict {
  param(
    [Parameter(Mandatory = $true)] $Recording1,
    [Parameter(Mandatory = $true)] $Recording2,
    $Segments = $null,
    $JobCheck = $null,
    [double]$OverlapSeconds = 0,
    [double]$MinOverlapSeconds = $script:MinOverlapSeconds
  )
  $fail = New-Object System.Collections.Generic.List[string]
  $invalid = New-Object System.Collections.Generic.List[string]
  foreach ($r in @($Recording1, $Recording2)) {
    foreach ($f in @($r.Fail)) { $fail.Add([string]$f) }
    foreach ($i in @($r.Invalid)) { $invalid.Add([string]$i) }
  }
  $checks = [ordered]@{}
  $checks['session 1 recording: Phase 1 rule, 0 dropped, 0 overruns, 0 unexcused xruns'] = ($Recording1.Status -eq 'PASS')
  $checks['session 2 recording: Phase 1 rule, 0 dropped, 0 overruns, 0 unexcused xruns'] = ($Recording2.Status -eq 'PASS')
  if ($null -eq $JobCheck) { $fail.Add('session 1: the queue report is missing, so the job cannot be judged'); $checks['session 1 transcript: job done, cursor at the end of the audio'] = $false }
  else {
    $checks['session 1 transcript: job done, cursor at the end of the audio'] = [bool]$JobCheck.Ok
    if (-not $JobCheck.Ok) { $fail.Add("session 1: $($JobCheck.Reason)") }
  }
  if ($null -eq $Segments) { $fail.Add('session 1: its committed segments were not read'); $checks['session 1 segments: no overlap, no duplicate, no gap, both ends covered'] = $false }
  else {
    $checks['session 1 segments: no overlap, no duplicate, no gap, both ends covered'] = [bool]$Segments.Ok
    foreach ($p in @($Segments.Problems)) { $fail.Add("session 1 segments: $p") }
  }
  $overlapOk = ($OverlapSeconds -ge $MinOverlapSeconds)
  $checks[("the queue decoded session 1 while session 2 recorded (at least {0:n0} s)" -f $MinOverlapSeconds)] = $overlapOk
  if (-not $overlapOk) { $invalid.Add(("the queue decoded session 1 for {0:n0} s while session 2 recorded (needs {1:n0} s): the run did not measure the recorder beside the queue" -f $OverlapSeconds, $MinOverlapSeconds)) }
  $status = $(if ($fail.Count -gt 0) { 'FAIL' } elseif ($invalid.Count -gt 0) { 'INVALID' } else { 'PASS' })
  [pscustomobject]@{ Status = $status; Fail = @($fail.ToArray()); Invalid = @($invalid.ToArray()); Checks = $checks }
}

# Lines for the evidence: the information items (no bar) and what the run cost.
function Get-WbQueueSoakInfoLines {
  param($LogInfo, [double]$OverlapSeconds = 0, $Segments = $null, $Job = $null)
  $l = New-Object System.Collections.Generic.List[string]
  $l.Add('information only (no bar):')
  if ($LogInfo -and $null -ne $LogInfo.EstimateErrorMinutes) {
    $l.Add(('  "ready by" estimate for session 1, given when it stopped ({0} on the app clock): {1}; the transcript was complete at {2}; error {3:+0;-0;0} min (clock minutes)' -f $LogInfo.EstimateGivenAt, $LogInfo.EstimatedReadyBy, $LogInfo.CompletedAt, $LogInfo.EstimateErrorMinutes))
  } else { $l.Add('  "ready by" estimate for session 1: not available (the log has no stop line with an estimate, or no completion line)') }
  if ($LogInfo -and $null -ne $LogInfo.EnginePeakMiB) { $l.Add(('  engine memory peak while session 1 was decoded: {0} MiB (the queue''s limit at its tightest: {1} MiB)' -f $LogInfo.EnginePeakMiB, $LogInfo.EngineLimitMiB)) }
  else { $l.Add('  engine memory peak: the log carries none') }
  $l.Add(('  decoding of session 1 inside the recording of session 2: {0:n0} s' -f $OverlapSeconds))
  if ($Segments) { $l.Add(('  session 1 transcript: {0} segments, largest gap {1:n1} s, first segment at {2}, last ends at {3} of {4:n1} s of audio' -f $Segments.Count, ($Segments.MaxGapMs / 1000.0), $(if ($null -ne $Segments.FirstStartMs) { ('{0:n1} s' -f ($Segments.FirstStartMs / 1000.0)) } else { 'n/a' }), $(if ($null -ne $Segments.LastEndMs) { ('{0:n1} s' -f ($Segments.LastEndMs / 1000.0)) } else { 'n/a' }), ($Segments.AudioMs / 1000.0))) }
  if ($Job) { $l.Add(('  job of session 1: state {0}, units {1} of {2}, cursor {3} of {4} ms' -f $Job.state, $Job.units_committed, $Job.units_total, $Job.cursor_ms, $Job.audio_ms)) }
  @($l.ToArray())
}

Import-Module (Join-Path $PSScriptRoot 'DaySim.psm1')
Export-ModuleMember -Function Get-WbQueueSoakGapLimitMs, Get-WbQueueSoakMinOverlapSeconds, Get-WbQueueSoakSettings, Get-WbQueueSoakCapability, Get-WbQueueSoakTitles, New-WbQueueSoakPlan, Test-WbQueueSoakSegments, Get-WbQueueSoakJob, Test-WbQueueSoakJobComplete, Get-WbDecodeOverlapSeconds, ConvertFrom-WbQueueUnitLine, ConvertTo-WbClockMinutes, Get-WbQueueSoakLogInfo, Get-WbQueueSoakRecordingVerdict, Get-WbQueueSoakVerdict, Get-WbQueueSoakInfoLines
