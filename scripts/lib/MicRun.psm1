<#
  MicRun.psm1: the pure parts of the gate-pack microphone run (gate mic-run).

  The run records the laptop's built-in microphone with the app (gate build) and two independent
  reference captures of the same input (cpal and raw WASAPI) while speech plays from the laptop's own
  speakers. Two scripted windows test the detectors: a DIGITAL MUTE of the microphone endpoint (the
  endpoint delivers zeros) while speech plays, and a ROOM SILENCE (the source itself is silent).
  Neither must count as a dropout. This module holds the schedule, the choice of the microphone and
  the loudspeaker, and the reading of the firings by window; the script does the starting and stopping.

  Windows PowerShell 5.1, ASCII only.
#>

Set-StrictMode -Version 2
Import-Module (Join-Path $PSScriptRoot 'PackCore.psm1')

# Pure. The schedule of one run, in seconds from the moment the source starts playing.
# Full: 120 minutes, a 3-minute mute at 40:00 and a 3-minute room silence at 80:00.
# Quick: 15 minutes, a 1-minute mute at 5:00 and a 1-minute room silence at 9:00.
function Get-PackMicPlan {
  [CmdletBinding()]
  param([bool]$Quick = $false)
  if ($Quick) { return [pscustomobject]@{ Seconds = 900; MuteAtS = 300; MuteForS = 60; SilenceAtS = 540; SilenceForS = 60; Quick = $true } }
  [pscustomobject]@{ Seconds = 7200; MuteAtS = 2400; MuteForS = 180; SilenceAtS = 4800; SilenceForS = 180; Quick = $false }
}

# Pure. The windows of a plan, as objects with Name, Kind and the start and end on the SOURCE clock (ms).
function Get-PackMicWindows {
  [CmdletBinding()]
  param([Parameter(Mandatory = $true)] $Plan)
  @(
    [pscustomobject]@{ Name = 'digital mute'; Kind = 'mute'; StartMs = 1000.0 * $Plan.MuteAtS; EndMs = 1000.0 * ($Plan.MuteAtS + $Plan.MuteForS) },
    [pscustomobject]@{ Name = 'room silence'; Kind = 'silence'; StartMs = 1000.0 * $Plan.SilenceAtS; EndMs = 1000.0 * ($Plan.SilenceAtS + $Plan.SilenceForS) }
  )
}

# Pure. The built-in microphone among the testbench's inputs (objects with name, is_default). A name
# that says microphone or array wins; the default input is the second choice; virtual cables, stereo
# mixes and loopbacks never. Returns the input object or $null.
function Select-PackMicDevice {
  [CmdletBinding()]
  param([Parameter(Mandatory = $true)] $Inputs)
  $bad = 'CABLE|VB-Audio|Virtual|Stereo Mix|Voicemeeter|Loopback|Line In|Line \(|Focusrite|Scarlett|USB Audio'
  $cands = @(@($Inputs) | Where-Object { $_ -and ([string]$_.name) -notmatch $bad })
  $mic = @($cands | Where-Object { ([string]$_.name) -match 'Microphone|Mic\b|Mic Array|Array' } | Sort-Object { if ($_.is_default) { 0 } else { 1 } } | Select-Object -First 1)
  if ($mic.Count -gt 0) { return $mic[0] }
  $def = @($cands | Where-Object { $_.is_default } | Select-Object -First 1)
  if ($def.Count -gt 0) { return $def[0] }
  $null
}

# Pure. Detector firings (Kind, AudioMs, LenMs on the APP clock) against the windows (source clock).
# OffsetMs: the app clock minus the source clock (the app started recording earlier, so it is the
# time the source took to start on the app's clock). Slack widens each window on both sides. Returns
# one row per window and one for the rest, each with the count per detector kind.
function Get-PackWindowTreatment {
  [CmdletBinding()]
  param($Firings = @(), [Parameter(Mandatory = $true)] $Windows, [double]$OffsetMs = 0, [double]$SlackMs = 1500)
  $f = @($Firings)
  $kinds = @('zero_gap', 'repeated_block')
  $inAny = New-Object 'System.Collections.Generic.HashSet[int]'
  $rows = New-Object System.Collections.Generic.List[object]
  foreach ($w in @($Windows)) {
    $lo = [double]$w.StartMs + $OffsetMs - $SlackMs
    $hi = [double]$w.EndMs + $OffsetMs + $SlackMs
    $row = [ordered]@{ Window = [string]$w.Name; Kind = [string]$w.Kind; AppStartMs = [double]$w.StartMs + $OffsetMs; AppEndMs = [double]$w.EndMs + $OffsetMs }
    foreach ($k in $kinds) { $row[$k] = 0 }
    for ($i = 0; $i -lt $f.Count; $i++) {
      $len = 0.0
      if ($f[$i].PSObject.Properties['LenMs'] -and $null -ne $f[$i].LenMs) { $len = [double]$f[$i].LenMs }
      $s = [double]$f[$i].AudioMs
      if (($s + $len) -ge $lo -and $s -le $hi) {
        $row[[string]$f[$i].Kind] = [int]$row[[string]$f[$i].Kind] + 1
        [void]$inAny.Add($i)
      }
    }
    $rows.Add([pscustomobject]$row)
  }
  $rest = [ordered]@{ Window = 'outside both windows'; Kind = 'rest'; AppStartMs = $null; AppEndMs = $null }
  foreach ($k in $kinds) { $rest[$k] = 0 }
  for ($i = 0; $i -lt $f.Count; $i++) { if (-not $inAny.Contains($i)) { $rest[[string]$f[$i].Kind] = [int]$rest[[string]$f[$i].Kind] + 1 } }
  $rows.Add([pscustomobject]$rest)
  @($rows.ToArray())
}

# Pure. The evidence lines for the windows: how each detector treated each one, in words.
function Get-PackWindowLines {
  [CmdletBinding()]
  param([Parameter(Mandatory = $true)] $Treatment)
  $lines = New-Object System.Collections.Generic.List[string]
  foreach ($r in @($Treatment)) {
    $when = $(if ($null -ne $r.AppStartMs) { ('app time {0:n0} s to {1:n0} s' -f ($r.AppStartMs / 1000.0), ($r.AppEndMs / 1000.0)) } else { 'the rest of the recording' })
    $lines.Add(('  {0} ({1}): zero_gap fired {2} time(s), repeated_block fired {3} time(s)' -f $r.Window, $when, $r.zero_gap, $r.repeated_block))
    if ($r.Kind -eq 'mute' -and ($r.zero_gap + $r.repeated_block) -gt 0) { $lines.Add('    note: a detector fired during the digital mute; a mute is silence the user asked for, not a dropout, so these are false alarms to read') }
    if ($r.Kind -eq 'silence' -and ($r.zero_gap + $r.repeated_block) -gt 0) { $lines.Add('    note: a detector fired during the room silence; the room is quiet, not broken, so these are false alarms to read') }
  }
  @($lines.ToArray())
}

# The evidence file and the result of the microphone run, from what the run measured. Lives here (and not in the gate
# script) so a test can run it with synthetic input: a mistake in this part is found after 15 minutes of recording
# on the laptop otherwise (a loop variable `$l` once overwrote the list `$L`: PowerShell names ignore case).
function New-PackMicRunResult {
  [CmdletBinding()]
  param(
    [Parameter(Mandatory = $true)] $Ctx, [Parameter(Mandatory = $true)] [string]$GateDir, [Parameter(Mandatory = $true)] [string]$Run,
    [Parameter(Mandatory = $true)] [string]$Profile, [Parameter(Mandatory = $true)] $Minutes, [Parameter(Mandatory = $true)] [bool]$Quick,
    [Parameter(Mandatory = $true)] [string]$MicName, [string]$Speaker = '', [string]$InputInfo = '',
    $Cond = @(), [Parameter(Mandatory = $true)] $Plan, $WindowLog = @(), [Parameter(Mandatory = $true)] $RefVerdict, $Detectors = $null,
    $Treatment = @(), [int]$Overruns = 0, [int]$Xruns = 0, [Parameter(Mandatory = $true)] [string]$Status, [string]$Why = '', [string]$Threshold = ''
  )
  $rv = $RefVerdict
  $ev = Join-Path $GateDir 'mic-run.txt'
  $evLines = New-Object System.Collections.Generic.List[string]
  $evLines.Add('Microphone run (gate pack)')
  $evLines.Add(('machine: {0}; profile: {1}; date: {2}; {3} minutes{4}' -f $Ctx.machine, $Profile, (Get-Date -Format 'yyyy-MM-dd HH:mm'), $Minutes, $(if ($Quick) { ' (QUICK: the full run is 120 minutes)' } else { '' })))
  $evLines.Add(('microphone: {0}; format the recorder opens: {1}; speakers: {2}' -f $MicName, $InputInfo, $Speaker))
  foreach ($c in @($Cond)) { $evLines.Add("  $c") }
  $evLines.Add(('windows (source clock): digital mute {0} s for {1} s; room silence {2} s for {3} s' -f $Plan.MuteAtS, $Plan.MuteForS, $Plan.SilenceAtS, $Plan.SilenceForS))
  foreach ($w in @($WindowLog)) { $evLines.Add(('  window log: {0} at {1} s ({2})' -f $w['event'], $w['elapsed_s'], $(if ($w['ok']) { 'done' } else { 'FAILED to change the mute' }))) }
  $evLines.Add(('recorder: overrun samples {0}, xruns {1}' -f $Overruns, $Xruns))
  foreach ($rl in @($rv.Lines)) { $evLines.Add([string]$rl) }
  $evLines.Add(('reference verdict: {0} ({1}): {2}' -f $rv.Verdict, $rv.Class, $rv.Reason))
  if ($Detectors) { foreach ($dl in @($Detectors.Lines)) { $evLines.Add([string]$dl) } }
  $evLines.Add('what each detector did in each scripted window:')
  foreach ($wl in @(Get-PackWindowLines -Treatment @($Treatment))) { $evLines.Add([string]$wl) }
  $evLines.Add("audio and reports: $Run (not in the repository; the known-speech recording stays on this laptop and is not in the zip)")
  $evLines.Add("verdict: $Status")
  Set-Content -LiteralPath $ev -Value @($evLines.ToArray()) -Encoding UTF8
  $measured = [ordered]@{ minutes = $Minutes; microphone = $MicName; overrun_samples = $Overruns; xruns = $Xruns; reference_class = $rv.Class; windows = @(@($Treatment) | ForEach-Object { [ordered]@{ window = $_.Window; zero_gap = $_.zero_gap; repeated_block = $_.repeated_block } }) }
  if ($Detectors) { $measured['detectors'] = $Detectors.Rates }
  $notes = @(@($Cond) | ForEach-Object { [string]$_ })
  New-PackResult -Id 'mic-run' -Status $Status -Headline ("{0} (microphone run, {1} minutes)" -f $Why, $Minutes) -Measured $measured -Threshold $Threshold -Rig ("built-in microphone: $MicName") -Evidence @($ev, (Join-Path $GateDir 'mic-windows.json')) -Notes $notes -Quick $Quick
}

Export-ModuleMember -Function Get-PackMicPlan, Get-PackMicWindows, Select-PackMicDevice, Get-PackWindowTreatment, Get-PackWindowLines, New-PackMicRunResult
