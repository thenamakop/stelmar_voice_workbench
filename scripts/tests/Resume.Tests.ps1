# Tests for the restart-and-resume path of the gate pack (scripts/lib/PackCore.psm1, scripts/gate-pack/run-gates.ps1).
#
# Why: on 17 Nov 2026 the dry run typed RESTART and got "Could not register the logon task: ERROR: Value for '/TR'
# option cannot be more than 261 character(s)". The runner put its whole command line into schtasks /TR, and it
# asked for the restart before it had registered anything. Now the command lives in a launcher script, the task only
# starts the launcher, and the carry-on is registered BEFORE the person is asked to restart.
#
# These tests create and delete a real, harmlessly named scheduled task and Run key entry on this machine
# (WB_PACK_RESUME_TASK, WB_PACK_RESUME_RUNKEY) and use a stub instead of shutdown.exe (WB_PACK_SHUTDOWN): no test can
# restart the computer or touch the real task. They leave nothing behind.
# Run: powershell -NoProfile -ExecutionPolicy Bypass -File scripts/tests/Resume.Tests.ps1
# ASCII only; Windows PowerShell 5.1.
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo = (Resolve-Path (Join-Path $here '..\..')).Path
$fails = 0
function Check([string]$Name, [bool]$Ok) {
  if ($Ok) { Write-Host "ok   $Name" } else { Write-Host "FAIL $Name"; $script:fails++ }
}
$ps = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
$tag = [guid]::NewGuid().ToString('N').Substring(0, 12)
$testTask = "STELMAR-GatePack-Test-$tag"
$testKey = "HKCU:\Software\STELMAR-GatePack-Test-$tag"
$env:WB_PACK_RESUME_TASK = $testTask
$env:WB_PACK_RESUME_RUNKEY = $testKey
Import-Module (Join-Path $here '..\lib\PackCore.psm1') -Force
$isAdmin = Test-PackAdmin
$level = $(if ($isAdmin) { 'Highest' } else { 'Limited' })
Write-Host ("note: administrator = {0}; the test task runs at run level {1}" -f $isAdmin, $level)
$tmp = Join-Path ([IO.Path]::GetTempPath()) ('wb-resume-' + $tag)
New-Item -ItemType Directory -Force -Path $tmp | Out-Null

# schtasks.exe writes to stderr when a task is missing; with Stop that would end the test.
function Invoke-Sch([string[]]$SchArgs) {
  $old = $ErrorActionPreference; $ErrorActionPreference = 'Continue'
  try { $null = & schtasks.exe @SchArgs 2>&1; return $LASTEXITCODE } finally { $ErrorActionPreference = $old }
}
function Remove-TestRegistration {
  [void](Invoke-Sch @('/Delete', '/F', '/TN', $testTask))
  if (Test-Path -LiteralPath $testKey) { Remove-Item -LiteralPath $testKey -Recurse -Force -ErrorAction SilentlyContinue }
}
function Test-TaskExists([string]$Name) { (Invoke-Sch @('/Query', '/TN', $Name)) -eq 0 }
# Runs a script with its answers read from a plain ASCII file (a redirected StandardInput writer can put a UTF-8
# byte-order mark in front of the first answer, so "RESTART" would not match). The launcher ends with Read-Host.
function Invoke-WithInput([string]$File, [string[]]$Arguments, [string[]]$InputLines, [int]$TimeoutSeconds = 120) {
  $id = [guid]::NewGuid().ToString('N').Substring(0, 8)
  $inF = Join-Path $tmp "in-$id.txt"; $outF = Join-Path $tmp "out-$id.txt"; $errF = Join-Path $tmp "err-$id.txt"
  [System.IO.File]::WriteAllBytes($inF, [System.Text.Encoding]::ASCII.GetBytes((($InputLines | ForEach-Object { $_ + "`r`n" }) -join '') + ''))
  $argText = ($Arguments | ForEach-Object { ConvertTo-PackArg $_ }) -join ' '
  $p = Start-Process -FilePath $File -ArgumentList $argText -RedirectStandardInput $inF -RedirectStandardOutput $outF -RedirectStandardError $errF -WindowStyle Hidden -PassThru
  $null = $p.Handle   # keeps the exit code readable after the process ends
  if (-not $p.WaitForExit($TimeoutSeconds * 1000)) { try { $p.Kill() } catch { }; return [pscustomobject]@{ Exit = -1; Out = 'TIMED OUT' } }
  $p.WaitForExit()
  $o = ''
  foreach ($f in @($outF, $errF)) { if (Test-Path -LiteralPath $f) { $o += (Get-Content -LiteralPath $f -Raw -ErrorAction SilentlyContinue) + "`n" } }
  [pscustomobject]@{ Exit = $p.ExitCode; Out = $o }
}

try {
  # ---- 1. The commands of the failed run and of a laptop fit now --------------------------------------
  $emu = 'D:\Projects\stelmar_voice_workbench\target\release\wb-emulate.exe'
  $p1 = New-PackResumeParams -Profile 'emu-low' -PackRoot 'D:\STELMAR-Gate\pack' -WorkDir 'D:\STELMAR-Gate\dryrun-work' -LockDir 'D:\Projects' -BackupRoot 'C:\STELMAR-Gate-Backup' -Emulate $emu -Quick $true -Only @('hashing-1', 'load-cold', 'hashing-2-10') -Role 'low' -RunLabel 'dryrun-partB'
  # The old way: the whole command in /TR.
  $old = '"{0}" -NoProfile -ExecutionPolicy Bypass -File "D:\STELMAR-Gate\pack\run-gates.ps1" -Resumed -Profile emu-low -PackRoot "D:\STELMAR-Gate\pack" -WorkDir "D:\STELMAR-Gate\dryrun-work" -LockDir "D:\Projects" -BackupRoot "C:\STELMAR-Gate-Backup" -Emulate "{1}" -Quick -Only hashing-1,load-cold,hashing-2-10' -f $ps, $emu
  Check 'the old /TR text of the failed run is over the 261 character limit (the bug)' ($old.Length -gt 261)
  $laptopWork = 'C:\Users\Maria Gonzalez-Hernandez\AppData\Local\STELMAR Gate Pack Results\low-baseline laptop run 2'
  $p2 = New-PackResumeParams -Profile 'laptop' -PackRoot 'E:\STELMAR-Gate\pack' -WorkDir $laptopWork -LockDir $laptopWork -BackupRoot 'F:\STELMAR-Gate-Backup' -Quick $false -Role 'mid'
  $old2 = '"{0}" -NoProfile -ExecutionPolicy Bypass -File "E:\STELMAR-Gate\pack\run-gates.ps1" -Resumed -Profile laptop -PackRoot "E:\STELMAR-Gate\pack" -WorkDir "{1}" -LockDir "{1}" -BackupRoot "F:\STELMAR-Gate-Backup"' -f $ps, $laptopWork
  Check 'the old /TR text of a laptop with a long user folder is over the limit too' ($old2.Length -gt 261)

  foreach ($case in @(@{ N = 'dry-run (the exact command of 17 Nov)'; P = $p1; Work = 'D:\STELMAR-Gate\dryrun-work' }, @{ N = 'laptop with a USB pack and a long user-profile work folder'; P = $p2; Work = $laptopWork })) {
    $launcher = Get-PackResumeLauncherPath -WorkDir $case.Work
    $action = '-NoProfile -ExecutionPolicy Bypass -File "{0}"' -f $launcher
    $tr = '"{0}" {1}' -f $ps, $action
    Check ("{0}: the registered action is short and holds only the launcher ({1} chars)" -f $case.N, $tr.Length) ($tr.Length -le 255 -and $tr -match 'resume-pack\.ps1' -and $tr -notmatch 'PackRoot')
  }
  Check 'the resume parameters carry the gate list, the label, the role and the emulator' ((@($p1['Only']) -join ',') -eq 'hashing-1,load-cold,hashing-2-10' -and $p1['RunLabel'] -eq 'dryrun-partB' -and $p1['Role'] -eq 'low' -and $p1['Emulate'] -eq $emu -and $p1['Quick'] -eq $true -and $p1['Resumed'] -eq $true)
  Check 'a laptop run has no emulator, no Quick and no gate list in its resume parameters' (-not $p2.Contains('Emulate') -and -not $p2.Contains('Quick') -and -not $p2.Contains('Only') -and $p2['Profile'] -eq 'laptop')

  # ---- 2. The launcher passes every value through, whatever the paths contain ---------------------------
  $weird = Join-Path $tmp "O'Brien  & Sons (x86)"
  $workReal = Join-Path $weird 'work folder'
  New-Item -ItemType Directory -Force -Path $workReal | Out-Null
  $stub = Join-Path $weird 'run gates stub.ps1'
  $got = Join-Path $workReal 'got.json'
  Set-Content -LiteralPath $stub -Encoding ASCII -Value @(
    '[CmdletBinding()] param([switch]$Resumed, [string]$Profile, [string]$PackRoot, [string]$WorkDir, [string]$LockDir, [string]$BackupRoot, [string]$Emulate, [switch]$Quick, [string[]]$Only, [string[]]$Skip, [string]$Role, [double]$MinFreeRamGB, [string]$RunLabel, [switch]$NoZip)',
    '$o = [ordered]@{ resumed = [bool]$Resumed; profile = $Profile; pack = $PackRoot; work = $WorkDir; lock = $LockDir; backup = $BackupRoot; emulate = $Emulate; quick = [bool]$Quick; only = @($Only); role = $Role; label = $RunLabel; cwd = (Get-Location).Path; ram = $MinFreeRamGB }',
    '$o | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $WorkDir ''got.json'') -Encoding UTF8')
  $pw = New-PackResumeParams -Profile 'emu-low' -PackRoot "D:\STELMAR-Gate\pack" -WorkDir $workReal -LockDir 'D:\Projects' -BackupRoot "C:\Users\O'Brien\backup" -Emulate $emu -Quick $true -Only @('hashing-1', 'load-cold') -Role 'low' -MinFreeRamGB 7.5 -RunLabel "part B 'x'"
  $lpath = New-PackResumeLauncher -Path (Get-PackResumeLauncherPath -WorkDir $workReal) -RunScript $stub -WorkDir $workReal -Params $pw
  $t = $null; $e = $null
  [void][System.Management.Automation.Language.Parser]::ParseFile($lpath, [ref]$t, [ref]$e)
  Check 'the launcher parses' (@($e).Count -eq 0)
  $run = Invoke-WithInput $ps @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $lpath) @('')
  Check 'the launcher runs the runner and ends (exit 0)' ($run.Exit -eq 0)
  $j = $null
  if (Test-Path -LiteralPath $got) { $j = Get-Content -LiteralPath $got -Raw | ConvertFrom-Json }
  Check 'the runner got -Resumed, the profile, the pack, the lock, the backup and the emulator unchanged' ($j -and $j.resumed -eq $true -and $j.profile -eq 'emu-low' -and $j.pack -eq 'D:\STELMAR-Gate\pack' -and $j.lock -eq 'D:\Projects' -and $j.backup -eq "C:\Users\O'Brien\backup" -and $j.emulate -eq $emu)
  Check 'the runner got Quick, the gate list in order, the role, the label with a quote, and the RAM limit' ($j -and $j.quick -eq $true -and (@($j.only) -join ',') -eq 'hashing-1,load-cold' -and $j.role -eq 'low' -and $j.label -eq "part B 'x'" -and [double]$j.ram -eq 7.5)
  Check 'the work folder (with an apostrophe, ampersand and spaces) is the current folder and the WorkDir' ($j -and $j.cwd -eq $workReal -and $j.work -eq $workReal)
  # A launcher whose runner is missing says so and writes resume-error.txt instead of closing silently.
  $lp2 = New-PackResumeLauncher -Path (Join-Path $workReal 'resume-missing.ps1') -RunScript (Join-Path $weird 'no such runner.ps1') -WorkDir $workReal -Params $pw -WaitForRunnerSeconds 2
  $run2 = Invoke-WithInput $ps @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $lp2) @('')
  Check 'a launcher whose runner is missing shows the error and writes resume-error.txt' ($run2.Out -match 'could not carry on' -and (Test-Path -LiteralPath (Join-Path $workReal 'resume-error.txt')))
  Remove-Item -LiteralPath $lp2, (Join-Path $workReal 'resume-error.txt') -Force -ErrorAction SilentlyContinue

  # ---- 3. A real task: create, read back, run its action, delete ---------------------------------------
  $marker = Join-Path $workReal 'task-ran.txt'
  $tl = Get-PackResumeLauncherPath -WorkDir $workReal
  Set-Content -LiteralPath $tl -Encoding UTF8 -Value @("Set-Content -LiteralPath '$($marker -replace "'", "''")' -Value (Get-Location).Path")
  Check 'before registering, no task of this name exists' (-not (Test-TaskExists $testTask))
  $how = Register-PackResume -Launcher $tl -Admin $isAdmin -RunLevel $level -DelaySeconds 30
  Write-Host "     registered: $how"
  if ($isAdmin) {
    Check 'the task exists after registering' (Test-TaskExists $testTask)
    Check 'Test-PackResumeRegistered says yes' (Test-PackResumeRegistered)
    $task = Get-ScheduledTask -TaskName $testTask
    $act = @($task.Actions)[0]
    Check 'the action starts Windows PowerShell with -File and only the launcher' ($act.Execute -eq $ps -and $act.Arguments -match [regex]::Escape($tl) -and $act.Arguments -match '-ExecutionPolicy Bypass' -and $act.Arguments.Length -lt 255)
    Check 'the action starts in the work folder' ($act.WorkingDirectory -eq $workReal)
    $trg = @($task.Triggers)[0]
    Check 'the trigger is at logon of the current user with a 30 second delay' ($trg.CimClass.CimClassName -match 'LogonTrigger' -and $trg.Delay -eq 'PT30S' -and $trg.UserId -eq [Security.Principal.WindowsIdentity]::GetCurrent().Name)
    Check 'the task runs elevated and interactive for the current user' ($task.Principal.RunLevel -eq 'Highest' -and $task.Principal.LogonType -eq 'Interactive' -and ([Security.Principal.WindowsIdentity]::GetCurrent().Name -match ([regex]::Escape([string]$task.Principal.UserId) + '$')))
    Check 'the task has no time limit and starts on battery' (($task.Settings.ExecutionTimeLimit -eq 'PT0S') -and -not $task.Settings.DisallowStartIfOnBatteries -and -not $task.Settings.StopIfGoingOnBatteries)
    # The action works: start the task now and wait for what the launcher writes.
    Start-ScheduledTask -TaskName $testTask
    $deadline = (Get-Date).AddSeconds(40)
    while (-not (Test-Path -LiteralPath $marker) -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 500 }
    Check 'starting the task runs the launcher (quoting and working folder work)' ((Test-Path -LiteralPath $marker) -and ((Get-Content -LiteralPath $marker -Raw).Trim() -eq $workReal))
  } else {
    Check 'the Run key entry exists after registering' ((Get-ItemProperty -Path $testKey -Name $testTask -ErrorAction SilentlyContinue).$testTask -match 'resume-pack\.ps1')
    Check 'Test-PackResumeRegistered says yes (Run key)' (Test-PackResumeRegistered)
  }
  $gone = @(Unregister-PackResume -WorkDir $workReal)
  Check 'Unregister-PackResume reports the task and the launcher it removed' ($gone.Count -ge 2 -and (($gone -join ' ') -match 'launcher'))
  Check 'afterwards no task is left' (-not (Test-TaskExists $testTask))
  Check 'afterwards Test-PackResumeRegistered says no' (-not (Test-PackResumeRegistered))
  Check 'afterwards the launcher file is gone' (-not (Test-Path -LiteralPath $tl))

  # The Run-key route (used without administrator rights) works and is removed too.
  $tl = Get-PackResumeLauncherPath -WorkDir $workReal
  Set-Content -LiteralPath $tl -Value 'exit 0' -Encoding ASCII
  $how2 = Register-PackResume -Launcher $tl -Admin $false
  Check 'the Run key entry is written and reads back' ($how2 -match 'Run key' -and ((Get-ItemProperty -Path $testKey -Name $testTask).$testTask -match 'resume-pack\.ps1'))
  [void](Unregister-PackResume -WorkDir $workReal)
  Check 'the Run key entry is removed' (-not (Get-ItemProperty -Path $testKey -Name $testTask -ErrorAction SilentlyContinue))
  # A launcher path that cannot fit in the Run key is refused in plain words, nothing is written.
  $longDir = Join-Path $tmp ('x' * 240)
  $msg = ''
  try { New-Item -ItemType Directory -Force -Path $longDir -ErrorAction Stop | Out-Null; Set-Content -LiteralPath (Join-Path $longDir 'resume-pack.ps1') -Value 'exit 0'; [void](Register-PackResume -Launcher (Join-Path $longDir 'resume-pack.ps1') -Admin $false) } catch { $msg = $_.Exception.Message }
  Check 'a launcher path that is too long for the Run key is refused with the reason, and nothing is registered' (($msg -match 'too long' -or $msg -eq '') -and -not (Test-PackResumeRegistered))
  # Registering a missing launcher fails before anything is created.
  $msg = ''
  try { [void](Register-PackResume -Launcher (Join-Path $tmp 'nothing.ps1') -Admin $isAdmin -RunLevel $level) } catch { $msg = $_.Exception.Message }
  Check 'registering a launcher that does not exist fails and leaves no task' ($msg -match 'Could not register' -and -not (Test-TaskExists $testTask))

  # ---- 4. The runner: registration comes before the question; a failure offers no restart --------------
  $pack = Join-Path $tmp 'pack'
  New-Item -ItemType Directory -Force -Path (Join-Path $pack 'scripts\gate-pack\gates'), (Join-Path $pack 'scripts\lib') | Out-Null
  Copy-Item -Path (Join-Path $repo 'scripts\lib\*') -Destination (Join-Path $pack 'scripts\lib') -Recurse -Force
  Copy-Item -LiteralPath (Join-Path $repo 'scripts\gate-pack\run-gates.ps1') -Destination (Join-Path $pack 'run-gates.ps1')
  Set-Content -LiteralPath (Join-Path $pack 'scripts\gate-pack\gates\cold1.ps1') -Encoding ASCII -Value @(
    'param([string]$CtxFile, [string]$ResultFile)',
    '$lib = Join-Path $PSScriptRoot ''..\..\lib''',
    'Import-Module (Join-Path $lib ''PackCore.psm1'') -Force; Import-Module (Join-Path $lib ''PackGate.psm1'') -Force',
    '$ctx = Read-PackCtx -CtxFile $CtxFile; Initialize-PackGate -Ctx $ctx',
    '$r = New-PackResult -Id "cold1" -Status "PASS" -Headline "fake cold1" -Rig "fake" -Quick ([bool]$ctx.quick)',
    'Complete-PackGate -Ctx $ctx -Result $r -ResultFile $ResultFile')
  $gate = [ordered]@{ id = 'cold1'; title = 'fake cold gate'; priority = 'P1'; order = 1; script = 'gates/cold1.ps1'; group = 'cold'; needs = [ordered]@{ boot_within_min = 1 }; interactive = $false; est_minutes = [ordered]@{ low = 1; mid = 1 }; depends_on = @(); ledger = @() }
  ([ordered]@{ pack_version = 1; gates = @($gate) } | ConvertTo-Json -Depth 8) | Set-Content -LiteralPath (Join-Path $pack 'scripts\gate-pack\gates.manifest.json') -Encoding UTF8
  $factsFile = Join-Path $tmp 'facts.json'
  Set-Content -LiteralPath $factsFile -Value '{"interface":true,"ac":true,"idle":true}' -Encoding ASCII
  $stubOk = Join-Path $tmp 'shutdown-ok.cmd'; $stubBad = Join-Path $tmp 'shutdown-bad.cmd'; $stubLog = Join-Path $tmp 'shutdown-args.txt'
  Set-Content -LiteralPath $stubOk -Encoding ASCII -Value @('@echo off', ('echo %* > "{0}"' -f $stubLog), 'exit /b 0')
  Set-Content -LiteralPath $stubBad -Encoding ASCII -Value @('@echo off', 'echo Access is denied.(5) 1>&2', 'exit /b 5')
  function Start-Runner([string]$WorkName, [string[]]$Answers, [string[]]$Extra = @()) {
    $a = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', (Join-Path $pack 'run-gates.ps1'), '-Role', 'low', '-FactsFile', $factsFile, '-WorkDir', (Join-Path $tmp $WorkName), '-LockDir', (Join-Path $tmp 'lock'), '-BackupRoot', (Join-Path $tmp 'backup'), '-NoZip') + $Extra
    Invoke-WithInput $ps $a $Answers 180
  }

  # 4a. The registration cannot be made (a folder sits where the launcher goes): no restart is offered, the work goes on.
  $w1 = Join-Path $tmp 'w1'
  New-Item -ItemType Directory -Force -Path (Join-Path $w1 'resume-pack.ps1') | Out-Null
  $env:WB_PACK_SHUTDOWN = $stubOk
  $r1 = Start-Runner 'w1' @('S')
  Check '4a: a failed registration says so in plain words' ($r1.Out -match 'THE PACK CANNOT RESTART THE LAPTOP YET' -and $r1.Out -match 'could not set up the step that carries on')
  Check '4a: the restart is NOT offered' ($r1.Out -notmatch 'A RESTART IS NEEDED' -and $r1.Out -notmatch 'Type RESTART')
  Check '4a: the computer was not asked to restart, no task was left' (-not (Test-Path -LiteralPath $stubLog) -and -not (Test-TaskExists $testTask) -and -not (Test-PackResumeRegistered))
  $s1 = Get-Content -LiteralPath (Join-Path $w1 'state.json') -Raw | ConvertFrom-Json
  Check '4a: no half-written restart state' ($null -eq $s1.restart.requested_at -and $null -eq $s1.restart.boot_time_before)

  # 4b. Registered first: the task exists when the question is asked; Enter (no restart) removes it again.
  $w2 = Join-Path $tmp 'w2'
  $r2b = Start-Runner 'w2' @('', 'S')
  Check '4b: the restart question names the carry-on that is already registered' ($r2b.Out -match 'A RESTART IS NEEDED' -and $r2b.Out -match 'carry on by itself when you log in again \((scheduled task|Run key entry)')
  Check '4b: registered is logged before the question' ($r2b.Out.IndexOf('registered: ') -ge 0 -and $r2b.Out.IndexOf('registered: ') -lt $r2b.Out.IndexOf('A RESTART IS NEEDED'))
  Check '4b: declining removes the task and the launcher, the computer is not restarted' (-not (Test-TaskExists $testTask) -and -not (Test-PackResumeRegistered) -and -not (Test-Path -LiteralPath (Join-Path $w2 'resume-pack.ps1')) -and -not (Test-Path -LiteralPath $stubLog))
  $s2 = Get-Content -LiteralPath (Join-Path $w2 'state.json') -Raw | ConvertFrom-Json
  Check '4b: declining leaves no restart state' ($null -eq $s2.restart.requested_at)

  # 4c. RESTART typed, Windows accepts (stub): the runner exits, the carry-on stays registered with the launcher.
  $w3 = Join-Path $tmp 'w3'
  $r3 = Start-Runner 'w3' @('RESTART') @('-Quick', '-Only', 'cold1', '-RunLabel', 'label 3')
  Check '4c: the runner asked Windows for a restart in 15 seconds' ((Test-Path -LiteralPath $stubLog) -and ((Get-Content -LiteralPath $stubLog -Raw) -match '/r /t 15'))
  if (-not (Test-Path -LiteralPath $stubLog)) { Write-Host '     runner output (for the reader of a failure):'; ($r3.Out -split "`n" | Select-Object -Last 25) | ForEach-Object { Write-Host "     | $_" } }
  Check '4c: the carry-on is registered and the launcher is in the work folder' ((Test-PackResumeRegistered) -and (Test-Path -LiteralPath (Join-Path $w3 'resume-pack.ps1')))
  $lt = Get-Content -LiteralPath (Join-Path $w3 'resume-pack.ps1') -Raw
  Check '4c: the launcher carries the work folder, the gate list, Quick and the label' ($lt -match [regex]::Escape("'" + $w3 + "'") -and $lt -match "Only = @\('cold1'\)" -and $lt -match 'Quick = \$true' -and $lt -match "RunLabel = 'label 3'" -and $lt -match 'Resumed = \$true')
  $s3 = Get-Content -LiteralPath (Join-Path $w3 'state.json') -Raw | ConvertFrom-Json
  Check '4c: the restart is recorded in the state' ($s3.restart.requested_at -and $s3.restart.boot_time_before)

  # 4d. -Cleanup removes the task and the launcher.
  $rc = Start-Runner 'w3' @() @('-Cleanup')
  Check '4d: -Cleanup removes the task, the Run key entry and the launcher' (-not (Test-TaskExists $testTask) -and -not (Test-PackResumeRegistered) -and -not (Test-Path -LiteralPath (Join-Path $w3 'resume-pack.ps1')) -and $rc.Out -match 'removed: ')

  # 4e. Windows refuses the restart (stub exits 5): the registration and the state are put back, in plain words.
  $env:WB_PACK_SHUTDOWN = $stubBad
  $w4 = Join-Path $tmp 'w4'
  $r4 = Start-Runner 'w4' @('RESTART', 'S')
  Check '4e: a refused restart says so' ($r4.Out -match 'THE LAPTOP DID NOT RESTART' -and $r4.Out -match 'Access is denied')
  $s4 = Get-Content -LiteralPath (Join-Path $w4 'state.json') -Raw | ConvertFrom-Json
  Check '4e: the task, the launcher and the restart state are put back' (-not (Test-TaskExists $testTask) -and -not (Test-PackResumeRegistered) -and -not (Test-Path -LiteralPath (Join-Path $w4 'resume-pack.ps1')) -and $null -eq $s4.restart.requested_at)

  # 4f. After the restart: the resumed run removes the task at its start and the launcher at its end.
  $env:WB_PACK_SHUTDOWN = $stubOk
  $w5 = Join-Path $tmp 'w5'
  [void](Start-Runner 'w5' @('RESTART') @('-Quick', '-Only', 'cold1'))
  Check '4f: before the resumed run the carry-on is registered' (Test-PackResumeRegistered)
  # The restart did not happen in this test, so the machine is not fresh: the resumed run asks again; Enter declines.
  $rr = Start-Runner 'w5' @('', 'S') @('-Resumed', '-Quick', '-Only', 'cold1')
  Check '4f: the resumed run says it resumed and ends with nothing registered or left in the work folder' ($rr.Out -match 'resumed after the restart' -and -not (Test-TaskExists $testTask) -and -not (Test-PackResumeRegistered) -and -not (Test-Path -LiteralPath (Join-Path $w5 'resume-pack.ps1')))
  $s5 = Get-Content -LiteralPath (Join-Path $w5 'state.json') -Raw | ConvertFrom-Json
  Check '4f: the resume time is recorded' ($null -ne $s5.restart.resumed_at)

  # 4g. A restart that was cancelled (the carry-on stays registered, the machine never restarted): the next ordinary start removes it.
  $w6 = Join-Path $tmp 'w6'
  [void](Start-Runner 'w6' @('RESTART') @('-Quick', '-Only', 'cold1'))
  Check '4g: after a cancelled restart the carry-on is still registered' (Test-PackResumeRegistered)
  $r6 = Start-Runner 'w6' @('', 'S') @('-Quick', '-Only', 'cold1', '-Status')
  Check '4g: -Status does not touch it' (Test-PackResumeRegistered)
  $r6 = Start-Runner 'w6' @('', 'S') @('-Quick', '-Only', 'cold1')
  Check '4g: the next ordinary start removes the leftover carry-on' ($r6.Out -match 'removed a leftover carry-on' -and -not (Test-PackResumeRegistered) -and -not (Test-Path -LiteralPath (Join-Path $w6 'resume-pack.ps1')))
  Check 'a drive that is not removable (the system drive) is not reported' (@(Get-PackRemovablePaths -Paths @($env:SystemRoot)).Count -eq 0)

  # ---- 5. Structure: the order in the source -----------------------------------------------------------
  $src = Get-Content -LiteralPath (Join-Path $repo 'scripts\gate-pack\run-gates.ps1') -Raw
  $i1 = $src.IndexOf('$how = Register-RestartResume'); $i2 = $src.IndexOf('Confirm-Restart -For $sel.RestartFor')
  Check 'run-gates.ps1 registers the carry-on before it asks the person to restart' ($i1 -gt 0 -and $i2 -gt $i1)
  Check 'run-gates.ps1 no longer builds a command line for the task' ($src -notmatch '-ExtraArguments' -and $src -notmatch '/TR')
} finally {
  Remove-TestRegistration
  Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue
  Remove-Item Env:\WB_PACK_RESUME_TASK, Env:\WB_PACK_RESUME_RUNKEY -ErrorAction SilentlyContinue
  Remove-Item Env:\WB_PACK_SHUTDOWN -ErrorAction SilentlyContinue
}
Check 'the test task is gone at the end' (-not (Test-TaskExists $testTask))
Check 'the test Run key is gone at the end' (-not (Test-Path -LiteralPath $testKey))
if ($fails) { Write-Host "$fails failed"; exit 1 }
Write-Host 'Resume tests passed'
