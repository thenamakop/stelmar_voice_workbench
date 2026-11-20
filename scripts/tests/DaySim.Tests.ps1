# Tests for scripts/lib/DaySim.psm1 (the day simulation's stop rules) and the estimate check of scripts/gates/day_report.py.
# Run: powershell -NoProfile -ExecutionPolicy Bypass -File scripts/tests/DaySim.Tests.ps1
# ASCII only; Windows PowerShell 5.1.
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
Import-Module (Join-Path $here '..\lib\DaySim.psm1') -Force
$fails = 0
function Check([string]$Name, [bool]$Ok) {
  if ($Ok) { Write-Host "ok   $Name" } else { Write-Host "FAIL $Name"; $script:fails++ }
}
function Throws([scriptblock]$Block) { try { & $Block; $false } catch { $true } }

$t0 = Get-Date '2026-11-20T10:00:00'

# ---- the session stops on the player's own end plus a margin, never on a timer from the start call ----
Check 'the margin after the player ends is at least 10 s' ((Get-WbDayStopMarginSeconds) -ge 10)
Check 'no stop while the player is still playing' (-not (Test-WbDaySessionStopDue -PlayerExitedAt $null -Now $t0.AddHours(3)))
Check 'no stop before the margin has passed' (-not (Test-WbDaySessionStopDue -PlayerExitedAt $t0 -Now $t0.AddSeconds(14) -MarginSeconds 15))
Check 'stop once the margin has passed' (Test-WbDaySessionStopDue -PlayerExitedAt $t0 -Now $t0.AddSeconds(15) -MarginSeconds 15)
Check 'a margin under 10 s is refused' (Throws { Test-WbDaySessionStopDue -PlayerExitedAt $t0 -Now $t0.AddSeconds(60) -MarginSeconds 9 })

# ---- the reference capture runs past the app's last frame ----
Check 'the reference delay is at least 2 s' ((Get-WbDayRefStopDelaySeconds) -ge 2)
Check 'no reference stop before the app has stopped' (-not (Test-WbDayRefStopDue -AppStoppedSeenAt $null -Now $t0.AddHours(1)))
Check 'no reference stop within 2 s of the app stopping' (-not (Test-WbDayRefStopDue -AppStoppedSeenAt $t0 -Now $t0.AddSeconds(2) -DelaySeconds 3))
Check 'reference stop 3 s after the app stopped' (Test-WbDayRefStopDue -AppStoppedSeenAt $t0 -Now $t0.AddSeconds(3) -DelaySeconds 3)
Check 'a reference delay under 2 s is refused' (Throws { Test-WbDayRefStopDue -AppStoppedSeenAt $t0 -Now $t0.AddSeconds(60) -DelaySeconds 1 })

# ---- the plan ----
$day = @(
  [pscustomobject]@{ title = 'Inaugural'; hall = 'Main Hall'; at_s = 0; seconds = 5411.2; languages = @('en', 'hi'); script = $null },
  [pscustomobject]@{ title = 'Parallel I'; hall = 'Hall A'; at_s = 7200; seconds = 5403.9; languages = @('en', 'hi'); script = $null },
  [pscustomobject]@{ title = 'Parallel II'; hall = 'Hall B'; at_s = 16200; seconds = 5458.985; languages = @('en', 'hi'); script = $null },
  [pscustomobject]@{ title = 'Panel'; hall = 'Main Hall'; at_s = 21600; seconds = 5403.6; languages = @('en', 'hi'); script = $null }
)
$plan = @(New-WbDayPlanSessions -Sessions $day -RunDir 'D:\run' -Device 'CABLE Output' -Programme $true)
Check 'four sessions are planned' ($plan.Count -eq 4)
Check 'every session has its own stop file' ((@($plan | ForEach-Object { $_.stop_file } | Select-Object -Unique)).Count -eq 4 -and $plan[2].stop_file -eq 'D:\run\STOP-SESSION-3')
Check 'the programme times hold where the audio fits the slot' ($plan[0].at_s -eq 0 -and $plan[1].at_s -eq 7200 -and $plan[2].at_s -eq 16200)
Check 'the last session is pushed past the end of the one before it (audio, lead-in room and margin)' ($plan[3].at_s -ge 16200 + 5459 + 90 + 20 -and $plan[3].programme_at_s -eq 21600)
Check 'the ceiling leaves 15 minutes after the audio (a safety net, not the way a session ends)' ($plan[0].duration_s -eq 5412 + 900)
Check 'a session pushed by more than 5 minutes is an error' (Throws {
  New-WbDayPlanSessions -Sessions @(
    [pscustomobject]@{ title = 'A'; hall = 'x'; at_s = 0; seconds = 9000; languages = @('en'); script = $null },
    [pscustomobject]@{ title = 'B'; hall = 'x'; at_s = 100; seconds = 60; languages = @('en'); script = $null }
  ) -RunDir 'D:\run' -Device 'd' -Programme $true })
$seq = @(New-WbDayPlanSessions -Sessions @($day[0], $day[1]) -RunDir 'D:\run' -Device 'd' -Programme $false)
Check 'without a programme the sessions follow one another from 0 with room for the player' ($seq[0].at_s -eq 0 -and $seq[1].at_s -ge 5412 + 90 + 30)

# The audio manifests of the smoke and kill runs carry no hall and no script: the plan still builds (a property a session
# does not have is null, strict mode must not throw).
$bare = @(New-WbDayPlanSessions -Sessions @([pscustomobject]@{ title = 'A'; at_s = 0; seconds = 60; languages = @('en') }) -RunDir 'D:un' -Device 'd' -Programme $false)
Check 'a session without a hall or a script is planned' ($bare.Count -eq 1 -and $null -eq $bare[0].hall -and $null -eq $bare[0].script)

# ---- the estimate check counts only sessions that had already stopped ----
$py = Get-Command python -ErrorAction SilentlyContinue
if ($py) {
  # unittest writes its progress to stderr; that is not an error here
  $ErrorActionPreference = 'Continue'
  $out = & $py.Source -I (Join-Path $here 'day_report_test.py') 2>&1
  $code = $LASTEXITCODE
  $ErrorActionPreference = 'Stop'
  Check 'day_report_test.py (the estimate check) passes' ($code -eq 0)
  if ($code -ne 0) { $out | ForEach-Object { Write-Host "     $_" } }
} else {
  Write-Host 'skip day_report_test.py (no python on the path)'
}

if ($fails) { Write-Host "$fails check(s) failed"; exit 1 }
Write-Host 'all checks passed'
