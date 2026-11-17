# Programs that disturb the measurements (Steam, LatencyMon, ...), 17 Nov 2026: the pack printed "Please close these programs" and
# carried on at once, so a helper at the laptop missed it. Now: at a start with a person present it asks them to close the programs
# and press Enter, looks again, asks once more, then goes on; after a restart (nobody present) it waits at most 10 minutes and
# goes on (never losing the one-hour window); both record which programs were running. Also the drive prompt no longer ends "C::".
# Run: powershell -NoProfile -ExecutionPolicy Bypass -File scripts/tests/ProgramCheck.Tests.ps1
# ASCII only; Windows PowerShell 5.1.
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo = (Resolve-Path (Join-Path $here '..\..')).Path
Import-Module (Join-Path $here '..\lib\PackCore.psm1') -Force
$fails = 0
function Check([string]$Name, [bool]$Ok) {
  if ($Ok) { Write-Host "ok   $Name" } else { Write-Host "FAIL $Name"; $script:fails++ }
}

# A fake machine: the programs are running for the first $script:openFor looks, then they are closed.
$script:looks = 0; $script:openFor = 0; $script:asked = New-Object System.Collections.ArrayList
$script:shown = New-Object System.Collections.ArrayList; $script:slept = New-Object System.Collections.ArrayList
$names = { $script:looks++; if ($script:looks -le $script:openFor) { @('Steam', 'steamwebhelper') } else { @() } }
$ask = { param($text) [void]$script:asked.Add($text) }
$show = { param($lines) foreach ($l in @($lines)) { [void]$script:shown.Add([string]$l) } }
$sleep = { param($s) [void]$script:slept.Add([int]$s) }
function Reset([int]$OpenFor) { $script:looks = 0; $script:openFor = $OpenFor; $script:asked.Clear(); $script:shown.Clear(); $script:slept.Clear() }
function Run-Check([hashtable]$P) { Invoke-PackProgramCheck @P -GetNames $names -Ask $ask -Show $show -Sleep $sleep }
function Total([object[]]$a) { $s = 0; foreach ($x in $a) { $s += [int]$x }; $s }

# --- a person is at the keyboard ---
Reset 0
$r = Run-Check @{ Attended = $true }
Check 'nothing running: no question, no list, outcome none' ($r.Outcome -eq 'none' -and $script:asked.Count -eq 0 -and $script:shown.Count -eq 0 -and $r.Initial.Count -eq 0)

Reset 2   # open at the first look (the start) and the second look (after the first Enter), closed at the third
$r = Run-Check @{ Attended = $true }
Check 'attended: the programs are listed by name and the person is asked to close them and press Enter' ($script:asked.Count -ge 1 -and ($script:shown -join ' ') -match 'Steam' -and ($script:shown -join ' ') -match 'steamwebhelper' -and $script:asked[0] -match 'press Enter')
Check 'attended: still running after the first Enter, so the person is asked once more' ($r.Asked -eq 2 -and $script:asked.Count -eq 2)

Reset 1   # open at the start only: closed by the time of the first check after Enter
$r = Run-Check @{ Attended = $true }
Check 'attended: closed after the first Enter, the pack goes on and the outcome is closed' ($r.Asked -eq 1 -and $r.Outcome -eq 'closed' -and $r.Final.Count -eq 0 -and ($r.Initial -join ',') -eq 'Steam,steamwebhelper')

Reset 50  # never closed
$r = Run-Check @{ Attended = $true }
Check 'attended: still running after two questions, it goes on and does not ask a third time' ($r.Asked -eq 2 -and $script:asked.Count -eq 2 -and $r.Outcome -eq 'still-running' -and ($r.Final -join ',') -eq 'Steam,steamwebhelper')
Check 'attended: it says it carries on and that the results will name the programs' (($script:shown -join ' ') -match 'Carrying on' -and ($script:shown -join ' ') -match 'results will say')

Reset 50
$r = Run-Check @{ Attended = $true; NoPrompt = $true }
Check '-NoPrompt lists the programs, asks nothing and goes on' ($script:asked.Count -eq 0 -and $r.Outcome -eq 'shown' -and ($script:shown -join ' ') -match 'Steam')

# --- after a restart: nobody is there ---
Reset 3   # the programs are closed on the fourth look: the start look plus three polls
$r = Run-Check @{ Attended = $false; WaitMinutes = 10; PollSeconds = 15 }
Check 'unattended: shows the list and never asks a question' ($script:asked.Count -eq 0 -and ($script:shown -join ' ') -match 'Steam' -and ($script:shown -join ' ') -match 'Waiting up to 10 minutes')
Check 'unattended: stops waiting as soon as they are closed (3 polls of 15 s)' ($r.Outcome -eq 'closed' -and (Total $script:slept) -eq 45 -and $r.WaitedMinutes -eq 0.8 -and $r.Final.Count -eq 0)

Reset 100000
$r = Run-Check @{ Attended = $false; WaitMinutes = 10; PollSeconds = 15 }
Check 'unattended: never waits longer than 10 minutes in all' ((Total $script:slept) -eq 600 -and $r.WaitedMinutes -eq 10 -and $r.Outcome -eq 'still-running')
Check 'unattended: it records what was still running and says it carries on' (($r.Final -join ',') -eq 'Steam,steamwebhelper' -and ($script:shown -join ' ') -match 'Carrying on')

Reset 100000
$r = Run-Check @{ Attended = $false; WaitMinutes = 1; PollSeconds = 40 }
Check 'unattended: the last sleep is cut so that the total is exactly the wait (40 + 20 s)' ((Total $script:slept) -eq 60 -and $script:slept.Count -eq 2)

Reset 100000
$r = Run-Check @{ Attended = $false; WaitMinutes = 0 }
Check 'unattended: a wait of 0 minutes does not sleep at all' ($script:slept.Count -eq 0 -and $r.Outcome -eq 'still-running')

# --- the names the check looks for ---
Check 'Steam, steamwebhelper, LatencyMon and RTSS are disturbing programs; an ordinary one is not' ((@(Get-PackBlockingPrograms -Names @('Steam', 'steamwebhelper', 'LatencyMon', 'RTSS', 'notepad', 'chrome')) -join ',') -eq 'LatencyMon,RTSS,Steam,steamwebhelper')

# --- the drive prompt ---
$pr = Get-PackBackupPrompt
Check 'the drive prompt does not end with a colon (Read-Host adds its own, which made "C::")' (-not $pr.TrimEnd().EndsWith(':') -and $pr -match 'press Enter to use C$')
$runner = [IO.File]::ReadAllText((Join-Path $repo 'scripts\gate-pack\run-gates.ps1'))
Check 'the runner takes the prompt from Get-PackBackupPrompt, with no literal prompt ending in "C:"' ($runner -match 'Read-Host \(Get-PackBackupPrompt\)' -and $runner -notmatch "Read-Host '[^']*:'")

# --- the runner and the README ---
Check 'the runner calls the program check, records the result in the state and in every gate' ($runner -match 'Invoke-PackProgramCheck' -and $runner -match "state\['programs'\]" -and $runner -match 'disturbing_programs' -and $runner -match 'disturbing-programs\.txt')
Check 'after a restart (-Resumed) and with -Unattended the check is unattended' ($runner -match '-Attended \(-not \(\$Unattended -or \$Resumed\)\)')
$readme = [IO.File]::ReadAllText((Join-Path $repo 'scripts\gate-pack\README.txt'))
Check 'the README tells the helper to switch off auto-start before the restart' ($readme -match 'start with Windows' -and $readme -match 'BEFORE the restart' -and $readme -match 'up to 10 minutes')
Check 'the README is plain ASCII' (-not ($readme.ToCharArray() | Where-Object { [int]$_ -gt 126 }))

if ($fails -gt 0) { Write-Host "$fails failed"; exit 1 }
Write-Host 'ProgramCheck tests passed'
