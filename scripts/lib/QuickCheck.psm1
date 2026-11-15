<#
  QuickCheck.psm1: the verdict of the quick-check gate, in PowerShell so the person at the laptop sees it
  (no Python on the laptop). Input: the report the gate-build app writes (WB_GATE_REPORT), the saved
  capability.json and the per-cell rows of ProcWatch. Pure functions; nothing is started or changed here.

  Rules (docs/GATES.md, Phase 3 quick check):
    - the check ends within the 480 s bar;
    - the app reported no error and was not stopped;
    - the recorder safety run passed (every entry), because live text and the hybrid choice depend on it;
    - capability.json was written and carries a decision.
  A partial check is expected on a low laptop (the processor cells and the night model are left for the
  full check) and is reported, not failed. The tier, configuration and per-language numbers are read
  out as information: they are what the laptop measured.

  Windows PowerShell 5.1, ASCII only.
#>

Set-StrictMode -Version 2

function Get-QProp {
  param($Object, [string]$Name, $Default = $null)
  if ($null -eq $Object) { return $Default }
  $p = $Object.PSObject.Properties[$Name]
  if ($null -eq $p) { return $Default }
  $p.Value
}

function Format-QNumber {
  param($Value, [string]$Format = '0.00')
  if ($null -eq $Value) { return 'n/a' }
  ([double]$Value).ToString($Format, [System.Globalization.CultureInfo]::InvariantCulture)
}

function Format-QConfig {
  param($Cfg)
  if ($null -eq $Cfg) { return '?' }
  $kind = Get-QProp $Cfg 'kind' '?'
  $idx = Get-QProp $Cfg 'index'
  $gpu = Get-QProp $Cfg 'gpu'
  if ($null -ne $idx) { return "$kind $idx" }
  if ($null -ne $gpu) { return "$kind gpu $gpu" }
  [string]$kind
}

# Returns Status (PASS / FAIL / INVALID), Headline, Problems (why not PASS), Lines (the readable
# evidence) and Numbers (what the gate records).
function Get-QuickCheckVerdict {
  [CmdletBinding()]
  param(
    $Report,
    $Capability,
    $Cells = @(),
    [double]$BarSeconds = 480
  )
  $problems = New-Object System.Collections.Generic.List[string]
  $invalid = New-Object System.Collections.Generic.List[string]
  $lines = New-Object System.Collections.Generic.List[string]

  if ($null -eq $Report) { $invalid.Add('the app wrote no report (it did not run the check or crashed)') }
  else {
    $err = Get-QProp $Report 'error'
    if ($err) { $invalid.Add(("the app reported an error: {0}" -f ([string]($err | ConvertTo-Json -Compress -Depth 3)))) }
    $stopped = Get-QProp $Report 'stopped'
    if ($stopped) { $invalid.Add(("the check was stopped before it finished: {0}" -f ([string]($stopped | ConvertTo-Json -Compress -Depth 3)))) }
  }
  if ($null -eq $Capability) { $invalid.Add('no capability.json was written') }

  $decision = $null
  $duration = $null
  $partial = $null
  if ($null -ne $Capability) {
    $check = Get-QProp $Capability 'check'
    $decision = Get-QProp $Capability 'decision'
    if ($null -ne $check) {
      $duration = Get-QProp $check 'duration_s'
      $partial = Get-QProp $check 'partial'
      $bar = Get-QProp $check 'bar_s'
      # The app writes bar_s 480 for both kinds of check; a bar the caller passed (the full check's 7200) wins.
      if ($null -ne $bar -and -not $PSBoundParameters.ContainsKey('BarSeconds')) { $BarSeconds = [double]$bar }
    }
    if ($null -eq $duration) { $invalid.Add('capability.json has no check time') }
    if ($null -eq $decision) { $invalid.Add('capability.json has no decision') }
  }

  if ($null -ne $duration) {
    if ([double]$duration -le $BarSeconds) { $lines.Add(("check time: {0} s against the {1} s bar: within" -f (Format-QNumber $duration '0'), (Format-QNumber $BarSeconds '0'))) }
    else {
      $lines.Add(("check time: {0} s against the {1} s bar: OVER" -f (Format-QNumber $duration '0'), (Format-QNumber $BarSeconds '0')))
      $problems.Add(("the quick check took {0} s; the bar is {1} s" -f (Format-QNumber $duration '0'), (Format-QNumber $BarSeconds '0')))
    }
  }
  if ($null -ne $partial) {
    $lines.Add(('partial: {0} (expected on a low laptop: the processor cells and the night model are left for the full check)' -f $(if ($partial) { 'yes' } else { 'no' })))
  }

  # Recorder safety: every entry must have passed.
  $safety = @()
  if ($null -ne $Capability) { $safety = @(Get-QProp $Capability 'recorder_safety' @()) }
  if ($safety.Count -eq 0) {
    $lines.Add('recorder safety run: none recorded')
    if ($null -ne $Capability -and $null -ne $decision) { $problems.Add('no recorder safety run is recorded in capability.json') }
  } else {
    foreach ($s in $safety) {
      $cfgText = Format-QConfig (Get-QProp $s 'config')
      $ok = [bool](Get-QProp $s 'passed' $false)
      $lines.Add(("recorder safety, {0}: {1}; {2} s, {3} overruns, {4} driver xruns" -f $cfgText, $(if ($ok) { 'passed' } else { 'FAILED' }), (Format-QNumber (Get-QProp $s 'duration_s') '0'), (Get-QProp $s 'overruns' '?'), (Get-QProp $s 'driver_xruns' '?')))
      if (-not $ok) { $problems.Add(("the recorder safety run failed for {0}" -f $cfgText)) }
    }
  }

  # What the laptop decided: information.
  if ($null -ne $decision) {
    $label = Get-QProp (Get-QProp $decision 'config') 'label'
    $lines.Add(("tier: {0}; transcribes on: {1}" -f (Get-QProp $decision 'tier' '?'), $(if ($label) { $label } else { '?' })))
    $langs = Get-QProp $decision 'languages'
    if ($null -ne $langs) {
      foreach ($l in $langs.PSObject.Properties) {
        $roll = Get-QProp $l.Value 'rolling'
        if ($null -ne $roll) {
          $lines.Add(("  {0}: {1}, RTF {2}, meets the bar: {3}, ready about {4} min after Stop" -f $l.Name, (Get-QProp $roll 'model' '?'), (Format-QNumber (Get-QProp $roll 'rtf') '0.000'), $(if (Get-QProp $roll 'meets_bar') { 'yes' } else { 'NO' }), (Format-QNumber (Get-QProp $roll 'ready_after_stop_min') '0.0')))
        }
      }
    }
    foreach ($v in @(Get-QProp $decision 'verdict' @())) { $lines.Add("  verdict line: $v") }
  }

  # Graphics adapter on Hindi: slower than 1.5x real time would exhaust the bar (gate pack item).
  $slowHindi = $null
  if ($null -ne $Capability) {
    $hiGpu = @(@(Get-QProp $Capability 'measurements' @()) | Where-Object {
        (Get-QProp $_ 'lang') -eq 'hi' -and (Get-QProp (Get-QProp $_ 'config') 'kind') -eq 'gpu' -and $null -ne (Get-QProp $_ 'rtf') })
    if ($hiGpu.Count -gt 0) {
      $worst = ($hiGpu | Measure-Object -Property rtf -Maximum).Maximum
      $slowHindi = ([double]$worst -gt 1.5)
      $lines.Add(("graphics adapter on Hindi: worst RTF {0}; slower than 1.5x real time: {1}" -f (Format-QNumber $worst '0.000'), $(if ($slowHindi) { 'YES (would exhaust the 480 s on the deciding cells)' } else { 'no' })))
    } else { $lines.Add('graphics adapter on Hindi: no measurement on a graphics adapter') }
  }

  # Per-cell times from the process watcher.
  $cellRows = @(@($Cells) | Where-Object { $_.Command -eq 'bench' })
  $lines.Add(("engine cells seen by the process watcher: {0}" -f $cellRows.Count))
  $cellTotal = 0.0
  $longest = 0.0
  foreach ($c in $cellRows) {
    $sec = $(if ($null -ne $c.Seconds) { [double]$c.Seconds } else { 0.0 })
    $cellTotal += $sec
    if ($sec -gt $longest) { $longest = $sec }
    $lines.Add(("  {0}  {1} {2} on {3}, {4} threads, {5}, {6}: {7} s" -f ([datetime]$c.Started).ToLocalTime().ToString('HH:mm:ss', [System.Globalization.CultureInfo]::InvariantCulture), $c.Model, $c.Lang, $c.Device, $c.Threads, $c.Mode, $c.Clip, (Format-QNumber $c.Seconds '0.0')))
  }
  if ($cellRows.Count -gt 0) {
    $share = ''
    if ($null -ne $duration -and [double]$duration -gt 0) { $share = (" ({0}% of the check; the rest is the recorder safety run, the hybrid test and waiting)" -f (Format-QNumber (100.0 * $cellTotal / [double]$duration) '0')) }
    $lines.Add(("cells together: {0} s{1}; longest cell: {2} s" -f (Format-QNumber $cellTotal '0'), $share, (Format-QNumber $longest '0')))
  } else { $lines.Add('no engine cell was seen: the per-cell times are missing (the process watcher saw nothing)') }

  # The hybrid test: the graphics engine alone on the excerpt of its half of the test, then both engines together.
  if ($null -ne $Capability) {
    $hyb = @(Get-QProp $Capability 'hybrid' @() | Where-Object { $null -ne $_ })
    foreach ($h in $hyb) {
      $galone = Get-QProp $h 'gpu_alone_rtf'
      $lines.Add(('hybrid test ({0}, graphics adapter {1}, {2} CPU threads): graphics engine alone RTF {3}; together: graphics {4}, CPU {5}' -f (Get-QProp $h 'lang'), (Get-QProp $h 'gpu'), (Get-QProp $h 'cpu_threads'), $(if ($null -ne $galone) { Format-QNumber $galone '0.000' } else { 'not measured' }), (Format-QNumber (Get-QProp $h 'gpu_rtf_concurrent') '0.000'), (Format-QNumber (Get-QProp $h 'cpu_rtf_concurrent') '0.000')))
    }
    if ($hyb.Count -eq 0) { $lines.Add('hybrid test: none recorded (the configuration chosen has no CPU-plus-graphics test)') }
    if ($null -ne $duration) { $lines.Add(('the whole check took {0} s; {1} s of the {2} s bar remained' -f (Format-QNumber $duration '0'), (Format-QNumber ([math]::Max(0.0, [double]$BarSeconds - [double]$duration)) '0'), (Format-QNumber $BarSeconds '0'))) }
  }

  $status = 'PASS'
  $headline = ''
  if ($invalid.Count -gt 0) { $status = 'INVALID'; $headline = ($invalid -join '; ') }
  elseif ($problems.Count -gt 0) { $status = 'FAIL'; $headline = ($problems -join '; ') }
  else {
    $headline = ('the quick check took {0} s (bar {1} s); recorder safety run passed; tier {2}' -f (Format-QNumber $duration '0'), (Format-QNumber $BarSeconds '0'), (Get-QProp $decision 'tier' '?'))
  }
  [pscustomobject]@{
    Status = $status; Headline = $headline; Problems = @($problems) + @($invalid); Lines = @($lines)
    Numbers = [ordered]@{
      check_seconds = $duration; bar_seconds = $BarSeconds; partial = $partial
      cells = $cellRows.Count; cells_total_seconds = [math]::Round($cellTotal, 1); longest_cell_seconds = [math]::Round($longest, 1)
      tier = (Get-QProp $decision 'tier'); gpu_hindi_slower_than_1_5 = $slowHindi
    }
  }
}

# Pure. The quick check (short excerpts) against the full check (whole bench clips) on the same rolling
# cells: the relative difference of the real-time factors, flagged above 10 percent, and marked DECIDING
# where the cell is the first-choice rolling model of a language. Inputs are the two capability.json
# objects. Returns Rows, Flagged, DecidingFlagged, Compared and Lines.
function Get-QuickFullComparison {
  [CmdletBinding()]
  param($Quick, $Full, [double]$Flag = 0.10)
  $lines = New-Object System.Collections.Generic.List[string]
  $rows = New-Object System.Collections.Generic.List[object]
  $empty = [pscustomobject]@{ Rows = @(); Flagged = 0; DecidingFlagged = 0; Compared = 0; Lines = @('not compared: one of the two runs is missing') }
  if ($null -eq $Quick -or $null -eq $Full) { return $empty }
  $key = {
    param($m)
    $c = Get-QProp $m 'config'
    $kind = [string](Get-QProp $c 'kind')
    $idx = Get-QProp $c 'index' (Get-QProp $c 'gpu')
    $cfg = $(if ($kind -eq 'gpu' -or $kind -eq 'hybrid') { "${kind}:$idx" } else { $kind })
    '{0}|{1}|{2}|{3}' -f (Get-QProp $m 'model'), (Get-QProp $m 'lang'), (Get-QProp $m 'mode'), $cfg
  }
  $cfgOf = {
    param($c)
    $kind = [string](Get-QProp $c 'kind')
    $idx = Get-QProp $c 'index' (Get-QProp $c 'gpu')
    $(if ($kind -eq 'gpu' -or $kind -eq 'hybrid') { "${kind}:$idx" } else { $kind })
  }
  # DECIDING: the first-choice rolling model of a language on the configuration the check chose, not the same model elsewhere.
  $deciding = @{}
  foreach ($cap in @($Quick, $Full)) {
    $dec = Get-QProp $cap 'decision'
    $chosen = & $cfgOf (Get-QProp (Get-QProp $dec 'config') 'config')
    $langs = Get-QProp $dec 'languages'
    if ($null -eq $langs) { continue }
    foreach ($l in $langs.PSObject.Properties) {
      $r = Get-QProp $l.Value 'rolling'
      if ($null -ne $r) { $deciding[('{0}|{1}|{2}' -f (Get-QProp $r 'model'), $l.Name, $chosen)] = $true }
    }
  }
  $qmap = @{}
  foreach ($m in @(Get-QProp $Quick 'measurements' @())) { $qmap[(& $key $m)] = $m }
  $flagged = 0
  $decFlagged = 0
  $compared = 0
  $lines.Add(('quick check excerpt {0} s; full check excerpt {1} s; flag above {2}%' -f (Format-QNumber (Get-QProp (Get-QProp $Quick 'check') 'excerpt_seconds') '0'), (Format-QNumber (Get-QProp (Get-QProp $Full 'check') 'excerpt_seconds') '0'), (Format-QNumber (100 * $Flag) '0')))
  foreach ($m in @(Get-QProp $Full 'measurements' @())) {
    if ((Get-QProp $m 'mode') -ne 'rolling') { continue }
    $q = $qmap[(& $key $m)]
    if ($null -eq $q) { continue }
    $qr = Get-QProp $q 'rtf'
    $fr = Get-QProp $m 'rtf'
    if ($null -eq $qr -or $null -eq $fr -or [double]$fr -le 0) { continue }
    $compared++
    $diff = [double]$qr / [double]$fr - 1.0
    $isFlag = ([math]::Abs($diff) -gt $Flag)
    $isDec = $deciding.ContainsKey(('{0}|{1}|{2}' -f (Get-QProp $m 'model'), (Get-QProp $m 'lang'), (& $cfgOf (Get-QProp $m 'config'))))
    $qClip = Format-QNumber (Get-QProp $q 'clip_seconds') '0'
    $fClip = Format-QNumber (Get-QProp $m 'clip_seconds') '0'
    $mark = ''
    if ($isFlag) { $flagged++; $mark = '  FLAG'; if ($isDec) { $decFlagged++; $mark += ' DECIDING' } }
    $rows.Add([pscustomobject]@{ Cell = (& $key $m); QuickRtf = [double]$qr; FullRtf = [double]$fr; Diff = $diff; Flag = $isFlag; Deciding = $isDec })
    $lines.Add(('  {0}  quick {1} ({2} s excerpt)  full {3} ({4} s excerpt)  {5}%{6}' -f (& $key $m), (Format-QNumber $qr '0.000'), $qClip, (Format-QNumber $fr '0.000'), $fClip, (Format-QNumber (100 * $diff) '+0.0;-0.0'), $mark))
  }
  $lines.Add(('cells compared {0}; flagged {1}; flagged and deciding {2}' -f $compared, $flagged, $decFlagged))
  [pscustomobject]@{ Rows = @($rows.ToArray()); Flagged = $flagged; DecidingFlagged = $decFlagged; Compared = $compared; Lines = @($lines.ToArray()) }
}

Export-ModuleMember -Function Get-QuickCheckVerdict, Get-QuickFullComparison
