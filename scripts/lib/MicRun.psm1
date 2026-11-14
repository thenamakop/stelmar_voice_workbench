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

Export-ModuleMember -Function Get-PackMicPlan, Get-PackMicWindows, Select-PackMicDevice, Get-PackWindowTreatment, Get-PackWindowLines
