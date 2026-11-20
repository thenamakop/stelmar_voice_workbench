# The stop rules of the day simulation, kept apart from scripts/gates/day-sim.ps1 so that tests can check them
# (owner, 20 Nov 2026, after the day of 19-20 Nov 2026: the plan stopped each session on a timer started at the start
# call, the start call returned up to 46 s after the recorder began, so the app stopped before the player had finished).
# ASCII only; Windows PowerShell 5.1.
Set-StrictMode -Version Latest

# A session is stopped this long after the player's own end (the player process has exited), never less than 10 s.
$script:StopMarginSeconds = 15
# The reference capture of a session is stopped this long after the app's last frame (the app's own 'DAY: stopped'
# line is written after the recording has been closed), never less than 2 s.
$script:RefStopDelaySeconds = 3
# The app stops a session by itself after the plan's duration when no stop file has appeared: a safety net for a player
# that died, never the way a session normally ends. The day's evidence fails when it was used.
$script:CeilingExtraSeconds = 900
# The room the plan leaves between a session and the next one for the player to finish (the lead-in of the start
# call, the audio, the stop margin and a pause); the app starts the next session only after the previous one stopped.
$script:SpacingExtraSeconds = 90

function Get-WbDayStopMarginSeconds { $script:StopMarginSeconds }
function Get-WbDayRefStopDelaySeconds { $script:RefStopDelaySeconds }

# The session's stop file is written when the player has exited and the margin has passed.
#   $PlayerExitedAt: when this script saw the player exit (null while it is playing)
function Test-WbDaySessionStopDue {
  param($PlayerExitedAt, [datetime]$Now, [int]$MarginSeconds = 15)
  if ($MarginSeconds -lt 10) { throw 'the stop margin after the player ends is at least 10 s' }
  if ($null -eq $PlayerExitedAt) { return $false }
  return (($Now - [datetime]$PlayerExitedAt).TotalSeconds -ge $MarginSeconds)
}

# The reference capture's stop file is written once the app has stopped and the delay has passed.
#   $AppStoppedSeenAt: when this script read the app's 'DAY: stopped' line (null before that)
function Test-WbDayRefStopDue {
  param($AppStoppedSeenAt, [datetime]$Now, [int]$DelaySeconds = 3)
  if ($DelaySeconds -lt 2) { throw 'the reference capture runs at least 2 s past the app''s last frame' }
  if ($null -eq $AppStoppedSeenAt) { return $false }
  return (($Now - [datetime]$AppStoppedSeenAt).TotalSeconds -ge $DelaySeconds)
}

# A property of a session object, or null when it has none (the audio manifests of the smoke and kill runs carry no hall
# or script, and this module runs in strict mode).
function Get-WbDayProp {
  param($Object, [string]$Name)
  $p = $Object.PSObject.Properties[$Name]
  if ($p) { return $p.Value }
  return $null
}

# The plan's sessions: each starts no earlier than its programme time and no earlier than 20 s after the previous one
# could have ended (the audio, the room for the start call, the stop margin); a push of more than 5 minutes is an error.
# Each session also carries the path of its stop file and the safety ceiling (the audio plus 15 minutes).
#   $Sessions: objects with title, hall, at_s (programme time), seconds (audio length), languages, script
function New-WbDayPlanSessions {
  param(
    [Parameter(Mandatory = $true)] $Sessions,
    [Parameter(Mandatory = $true)] [string]$RunDir,
    [Parameter(Mandatory = $true)] [string]$Device,
    [bool]$Programme = $true
  )
  $planned = @()
  $prevEnd = -20
  $i = 0
  foreach ($s in @($Sessions)) {
    $i++
    $audio = [int][math]::Ceiling([double]$s.seconds)
    $ceiling = $audio + $script:CeilingExtraSeconds
    $room = $audio + $script:SpacingExtraSeconds
    $atEff = [int]$s.at_s
    if ($Programme) {
      if ($atEff -lt $prevEnd + 20) { $atEff = $prevEnd + 20 }
      if ($atEff - [int]$s.at_s -gt 300) { throw "the sessions overlap: '$($s.title)' is due at $($s.at_s) s, the one before it ends at $prevEnd s" }
    } else {
      $atEff = 0
      if ($i -gt 1) { $atEff = $prevEnd + 30 }
    }
    $planned += [ordered]@{
      title = $s.title; hall = (Get-WbDayProp $s 'hall'); at_s = $atEff; programme_at_s = [int]$s.at_s; duration_s = $ceiling
      stop_file = (Join-Path $RunDir "STOP-SESSION-$i"); languages = @($s.languages); script = (Get-WbDayProp $s 'script'); device = $Device
    }
    $prevEnd = $atEff + $room
  }
  return $planned
}

Export-ModuleMember -Function Get-WbDayStopMarginSeconds, Get-WbDayRefStopDelaySeconds, Test-WbDaySessionStopDue, Test-WbDayRefStopDue, New-WbDayPlanSessions
