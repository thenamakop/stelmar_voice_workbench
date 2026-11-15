<#
.SYNOPSIS
  Runs every pending STELMAR gate on this laptop and writes one zip of results.

.DESCRIPTION
  One command. It reads gates.manifest.json, works out what can run now (priority P1 first, then P2),
  runs it, saves its place after every gate, and asks you (in plain words) for anything it needs: the
  audio interface, mains power, a restart, the Wi-Fi switch. Results go to work\ and to
  gate-results-<machine>.zip next to this file.

    run-gates.ps1 -Role low        first run on the low-baseline laptop (use -Role mid on the other)
    run-gates.ps1                  carry on where it stopped
    run-gates.ps1 -Status          what is done, what is waiting, how long is left
    run-gates.ps1 -Only soak,kill  just these gates
    run-gates.ps1 -Retry           run failed gates again
    run-gates.ps1 -Quick           the short version used for the dry run on the Legion
    run-gates.ps1 -Verify          check every file of the pack against PACK-MANIFEST.json
    run-gates.ps1 -Cleanup         remove the logon task and put back volume, mute and power settings

  The runner never restarts the laptop by itself: it asks, and restarts only after you type RESTART.
  It never changes security settings. It needs no network and no development tools.

  Windows PowerShell 5.1, ASCII only.
#>
[CmdletBinding()]
param(
  [ValidateSet('low', 'mid', '')] [string]$Role = '',
  [switch]$Quick,
  [switch]$Status,
  [string[]]$Only = @(),
  [string[]]$Skip = @(),
  [switch]$Retry,
  [switch]$Verify,
  [switch]$Cleanup,
  [switch]$NoRestart,
  [switch]$Unattended,
  # With -Unattended: how long to keep checking for a missing interface, power or similar (default 20).
  [int]$UnattendedWaitMinutes = 20,
  [switch]$Resumed,
  # 'laptop' on the laptops. The dry run on the Legion uses 'emu-low' or 'emu-mid' with -Emulate.
  [ValidateSet('laptop', 'emu-low', 'emu-mid')] [string]$Profile = 'laptop',
  [string]$Emulate = '',
  [string]$PackRoot = '',
  [string]$WorkDir = '',
  [string]$LockDir = '',
  [string]$BackupRoot = '',
  [double]$MinFreeRamGB = 0,
  [string]$RunLabel = '',
  # For tests: the facts the runner would measure.
  [string]$FactsFile = '',
  [switch]$NoZip
)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
# With -File a comma list (-Only soak,kill) arrives as one string: split it.
$Only = @($Only | ForEach-Object { $_ -split ',' } | ForEach-Object { $_.Trim() } | Where-Object { $_ })
$Skip = @($Skip | ForEach-Object { $_ -split ',' } | ForEach-Object { $_.Trim() } | Where-Object { $_ })
if (-not $PackRoot) {
  if (Test-Path -LiteralPath (Join-Path $here 'scripts\gate-pack\gates.manifest.json')) { $PackRoot = $here }
  else { $PackRoot = (Resolve-Path (Join-Path $here '..\..')).Path }
}
$PackRoot = (Resolve-Path $PackRoot).Path
$gpDir = Join-Path $PackRoot 'scripts\gate-pack'
$libDir = Join-Path $PackRoot 'scripts\lib'
if (-not $WorkDir) { $WorkDir = Join-Path $PackRoot 'work' }
if (-not $LockDir) { $LockDir = $WorkDir }
New-Item -ItemType Directory -Force -Path $WorkDir | Out-Null
$WorkDir = (Resolve-Path $WorkDir).Path
Import-Module (Join-Path $libDir 'PsEnv.psm1') -Force
Set-WbCleanPSModulePath
Set-WbInvariantCulture
Import-Module (Join-Path $libDir 'PackCore.psm1') -Force
Set-PackLogFile (Join-Path $WorkDir 'runner.log')

$testbench = Join-Path $PackRoot 'target\release\wb-testbench.exe'
$manifestPath = Join-Path $gpDir 'gates.manifest.json'
$packManifestPath = Join-Path $PackRoot 'PACK-MANIFEST.json'
$isEmu = $Profile -ne 'laptop'
$runnerScript = Join-Path $PackRoot 'run-gates.ps1'
if (-not (Test-Path -LiteralPath $runnerScript)) { $runnerScript = $MyInvocation.MyCommand.Path }

function Say([string]$m) { Write-PackLog $m }
function Banner([string[]]$Lines, [string]$Colour = 'Yellow') {
  $w = 78
  Write-Host ''
  Write-Host ('=' * $w) -ForegroundColor $Colour
  foreach ($l in $Lines) { Write-Host $l -ForegroundColor $Colour }
  Write-Host ('=' * $w) -ForegroundColor $Colour
  foreach ($l in $Lines) { Add-Content -LiteralPath (Join-Path $WorkDir 'runner.log') -Value $l -Encoding UTF8 -ErrorAction SilentlyContinue }
}

# ---- -Cleanup ------------------------------------------------------------------------------------
if ($Cleanup) {
  $lines = @(Invoke-PackCleanup -WorkDir $WorkDir -Testbench $testbench)
  $left = @(Get-PackStillChanged -WorkDir $WorkDir)
  Banner (@('STELMAR gate pack: cleanup') + ($lines | ForEach-Object { '  ' + $_ }) + @('') + $(if ($left.Count -gt 0) { @('STILL CHANGED (put these back by hand):') + ($left | ForEach-Object { '  ' + $_ }) } else { @('Nothing is left changed by the pack.') })) $(if ($left.Count -gt 0) { 'Red' } else { 'Green' })
  if ($left.Count -gt 0) { exit 1 }
  exit 0
}

# ---- -Verify -------------------------------------------------------------------------------------
if ($Verify) {
  $pm = Read-PackJson -Path $packManifestPath
  if ($null -eq $pm) { Banner @('There is no PACK-MANIFEST.json here, so the pack cannot be verified.') 'Red'; exit 1 }
  $bad = New-Object System.Collections.Generic.List[string]
  $n = 0
  foreach ($f in @($pm.files)) {
    $n++
    $p = Join-Path $PackRoot ([string]$f.path -replace '/', '\')
    if (-not (Test-Path -LiteralPath $p)) { $bad.Add("missing: $($f.path)"); continue }
    $h = (Get-FileHash -LiteralPath $p -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($h -ne [string]$f.sha256) { $bad.Add("changed: $($f.path)") }
    if ($n % 50 -eq 0) { Say "checked $n of $(@($pm.files).Count) files" }
  }
  if ($bad.Count -gt 0) { Banner (@("The pack does not match PACK-MANIFEST.json ($($bad.Count) problem(s)):") + ($bad | Select-Object -First 30 | ForEach-Object { '  ' + $_ })) 'Red'; exit 1 }
  Banner @("All $n files match PACK-MANIFEST.json (built from commit $($pm.source_commit) on $($pm.built_at)).") 'Green'
  exit 0
}

# ---- The registry and the state -------------------------------------------------------------------
$manifest = Read-PackManifest -Path $manifestPath
$inPack = @(@($manifest.gates) | Where-Object { Test-Path -LiteralPath (Join-Path $gpDir ([string]$_.script -replace '/', '\')) } | ForEach-Object { [string]$_.id })
$state = Read-PackState -WorkDir $WorkDir
if ($null -eq $state) {
  if (-not $Role) {
    if ($Status -or $Unattended) { $Role = 'low' }
    else {
      $a = (Read-Host 'Is this the LOW-baseline laptop or the MID laptop? Type L or M').Trim().ToUpper()
      $Role = $(if ($a -eq 'M') { 'mid' } else { 'low' })
    }
  }
  $state = New-PackState -Role $Role -Profile $Profile -Quick ([bool]$Quick)
  Save-PackState -WorkDir $WorkDir -State $state
}
if ($Role -and $state['role'] -ne $Role) { Say "note: this work folder was started as the '$($state['role'])' laptop; keeping that."; }
$Role = [string]$state['role']
if ($Quick -and -not [bool]$state['quick']) { $state['quick'] = $true }
$quickRun = [bool]$state['quick']
if (-not $BackupRoot -and $state.Contains('backup_root')) { $BackupRoot = [string]$state['backup_root'] }

# What the facts file or the machine says, for the selection rule.
function Get-RunnerFacts {
  param($PendingNeeds)
  if ($FactsFile -and (Test-Path -LiteralPath $FactsFile)) { return (Read-PackJson -Path $FactsFile -AsHashtable) }
  $f = @{}
  $f['ac'] = Get-PackMainsPower
  $iface = $false
  try {
    if (Test-Path -LiteralPath $testbench) {
      Import-Module (Join-Path $libDir 'CableRig.psm1') -Force
      Import-Module (Join-Path $libDir 'LoopbackRig.psm1') -Force
      $dev = Get-PackTestbenchDevices -Testbench $testbench
      if ($isEmu) { $iface = (@(Get-WbCableEndpoints | Where-Object { $_.Active }).Count -gt 0) }
      else { $iface = (@(Get-PackLoopbackPairs -Devices $dev).Count -gt 0) -or (@(Get-WbCableEndpoints | Where-Object { $_.Active }).Count -gt 0) }
    }
  } catch { $iface = $false }
  $f['interface'] = $iface
  if ($PendingNeeds.Idle) { $f['idle'] = Test-PackIdle }
  $f
}

function Test-AnyPendingNeed {
  $idle = $false
  foreach ($g in @($manifest.gates)) {
    $gs = Get-PackGateState -State $state -Id ([string]$g.id)
    $st = [string](Get-PackProp $gs 'status' '')
    if (@('PASS', 'FAIL', 'INFO', 'SKIPPED') -contains $st) { continue }
    if ([bool](Get-PackProp (Get-PackProp $g 'needs' $null) 'idle' $false)) { $idle = $true }
  }
  [pscustomobject]@{ Idle = $idle }
}

# ---- -Status -------------------------------------------------------------------------------------
function Get-EstimateMinutes($g) {
  if ($quickRun) { $q = Get-PackProp $g 'quick' $null; if ($q -and (Get-PackProp $q 'skipped' $false)) { return 0 }; if ($q -and $null -ne (Get-PackProp $q 'minutes' $null)) { return [double]$q.minutes } }
  [double](Get-PackProp (Get-PackProp $g 'est_minutes' $null) $Role 30)
}
function Show-Status {
  Write-Host ''
  Write-Host ("STELMAR gate pack: {0} laptop, {1}{2}" -f $Role, $(if ($quickRun) { 'quick (dry run) ' } else { 'full length ' }), $(if ($isEmu) { "[emulated: $Profile]" } else { '' }))
  $remaining = 0.0
  foreach ($g in @($manifest.gates | Sort-Object { [string]$_.priority }, { [int]$_.order })) {
    $id = [string]$g.id
    $gs = Get-PackGateState -State $state -Id $id
    $st = [string](Get-PackProp $gs 'status' '')
    if ($inPack -notcontains $id) { $st = 'NOT IN THIS PACK' }
    elseif (-not $st) { $st = 'not run'; $remaining += (Get-EstimateMinutes $g) }
    elseif (@('PENDING', 'INVALID', 'interrupted', 'running') -contains $st) { $remaining += (Get-EstimateMinutes $g) }
    $why = [string](Get-PackProp $gs 'reason' '')
    Write-Host ('  {0} {1,-20} {2,-18} {3}' -f $g.priority, $id, $st, $why)
  }
  Write-Host ('  about {0} still to run' -f (Format-PackMinutes $remaining))
  Write-Host ''
}
if ($Status) { Show-Status; exit 0 }

# ---- Start: unblock, launch check, blockers -------------------------------------------------------
Banner @('STELMAR gate pack', "  laptop: $Role   profile: $Profile   $(if ($quickRun) { 'QUICK (dry run, not full length)' } else { 'full length' })", "  pack: $PackRoot", "  results: $WorkDir")
$ub = Invoke-PackUnblock -Root $PackRoot
Say ("unblocked {0} file(s) marked as downloaded" -f $ub.Unblocked)
if (@($ub.StillMarked).Count -gt 0) {
  Banner (@('Windows still marks these files as downloaded from the internet. The pack does not change security settings.', 'Right-click each file, Properties, tick Unblock, or copy the pack again from the USB drive:') + (@($ub.StillMarked) | Select-Object -First 8 | ForEach-Object { '  ' + $_ })) 'Red'
}
$blockedFiles = New-Object System.Collections.Generic.List[string]
foreach ($t in @(@{ P = (Join-Path $PackRoot 'src-tauri\binaries'); A = @('info') })) {
  if (Test-Path -LiteralPath $t.P) {
    foreach ($e in @(Get-ChildItem -LiteralPath $t.P -Filter 'wb-engine-*.exe' -File)) {
      if ($e.Name -match 'cuda|openvino') { continue }
      $r = Test-PackLaunchable -Path $e.FullName -Arguments $t.A -TimeoutSeconds 30
      if ($r.Blocked) { $blockedFiles.Add("$($e.Name): $($r.Message)") }
    }
  }
}
if (Test-Path -LiteralPath $testbench) {
  $r = Test-PackLaunchable -Path $testbench -Arguments @('devices') -TimeoutSeconds 30
  if ($r.Blocked) { $blockedFiles.Add("wb-testbench.exe: $($r.Message)") }
}
if ($blockedFiles.Count -gt 0) {
  Banner (@('WINDOWS OR YOUR ANTIVIRUS BLOCKED A FILE OF THE PACK. Nothing was changed.', 'Tell the pack owner which file and what Windows said:') + ($blockedFiles | ForEach-Object { '  ' + $_ }) + @('Do not turn protection off. Ask for an exception for the pack folder, or copy the pack again.')) 'Red'
}
$blockers = @(Get-PackBlockingPrograms)
if ($blockers.Count -gt 0) {
  Banner (@('Please close these programs before the recorder gates (they disturb the measurements):') + ($blockers | ForEach-Object { '  ' + $_ })) 'Yellow'
}
if (-not (Test-PackAdmin)) {
  Say 'note: not running as administrator. The cold-cache hashing runs and the logon task need it; those gates will say so.'
}

# A restart brought us back.
$uptime = (Get-PackBootInfo)
if ($Resumed) {
  $state['restart']['resumed_at'] = (Get-Date).ToString('o')
  $state['restart']['uptime_at_resume_min'] = [math]::Round($uptime.UptimeMin, 1)
  Say ("resumed after the restart: the laptop has been up {0:n1} minutes" -f $uptime.UptimeMin)
  Unregister-PackResume | Out-Null
}
# Anything that was running when the laptop restarted or the runner died was interrupted.
foreach ($k in @($state['gates'].Keys)) {
  if ([string]$state['gates'][$k]['status'] -eq 'running') { $state['gates'][$k]['status'] = 'interrupted'; $state['gates'][$k]['reason'] = 'interrupted (the laptop restarted or the runner stopped)' }
}
Save-PackState -WorkDir $WorkDir -State $state

# Where the recordings are mirrored: an external drive when there is one.
if (-not $BackupRoot) {
  if ($Unattended -or $FactsFile) { $BackupRoot = 'C:\STELMAR-Gate-Backup' }
  else {
    Write-Host ''
    Write-Host 'The recorder gates mirror every recording to a second place.'
    $d = (Read-Host 'Type the drive letter of the external SSD (for example E:) or press Enter to use C: (the results then say the mirror is on the same disk)').Trim()
    if ($d -match '^[A-Za-z]:?\\?$') { $BackupRoot = ($d.Substring(0, 1).ToUpper() + ':\STELMAR-Gate-Backup') } else { $BackupRoot = 'C:\STELMAR-Gate-Backup' }
  }
  $state['backup_root'] = $BackupRoot
  Save-PackState -WorkDir $WorkDir -State $state
}
if ($MinFreeRamGB -le 0) { $MinFreeRamGB = 7 }

$machine = Get-PackMachineName
$deferred = New-Object System.Collections.Generic.HashSet[string]

# ---- Running one gate -----------------------------------------------------------------------------
function Invoke-OneGate($g) {
  $id = [string]$g.id
  $gdir = Join-Path $WorkDir ("gates\$id")
  New-Item -ItemType Directory -Force -Path $gdir | Out-Null
  $resultFile = Join-Path $gdir 'result.json'
  if (Test-Path -LiteralPath $resultFile) { Remove-Item -LiteralPath $resultFile -Force }
  Remove-Item -LiteralPath (Join-Path $gdir 'PROMPT.txt'), (Join-Path $gdir 'ANSWER.txt') -Force -ErrorAction SilentlyContinue
  $up = Get-PackBootInfo
  $ctx = [ordered]@{
    pack_root = $PackRoot; work_dir = $WorkDir; gate_id = $id; gate_dir = $gdir; profile = $Profile; emulate = $Emulate
    role = $Role; quick = $quickRun; machine = $machine; lock_dir = $LockDir; min_free_ram_gb = $MinFreeRamGB
    uptime_min = [math]::Round($up.UptimeMin, 1); boot_time = $up.BootTime.ToString('o'); admin = (Test-PackAdmin)
    rig_prefer = $(if ($isEmu) { 'cable' } else { 'loopback' }); backup_root = $BackupRoot; unattended = [bool]$Unattended
    runner = $runnerScript
  }
  $ctxFile = Join-Path $gdir 'ctx.json'
  Write-PackJson -Path $ctxFile -Value $ctx
  $gs = Get-PackGateState -State $state -Id $id
  $attempts = [int](Get-PackProp $gs 'attempts' 0) + 1
  Set-PackGateStatus -State $state -Id $id -Fields @{ status = 'running'; attempts = $attempts; started = (Get-Date).ToString('o'); reason = ''; result_file = $resultFile }
  [void]$state['order_used'].Add($id)
  Save-PackState -WorkDir $WorkDir -State $state
  $est = Get-EstimateMinutes $g
  $timeout = [int]([math]::Max(30, $est * 2.5 + 30))
  Say ("START {0} {1}: {2} (about {3})" -f $g.priority, $id, $g.title, (Format-PackMinutes $est))
  $began = Get-Date
  $onTick = {
    param($proc)
    $pf = Join-Path $gdir 'PROMPT.txt'
    if (Test-Path -LiteralPath $pf) {
      $msg = (Get-Content -LiteralPath $pf -Raw -Encoding UTF8)
      Remove-Item -LiteralPath $pf -Force
      Banner (@("$id needs you:") + @($msg -split "`r?`n" | ForEach-Object { '  ' + $_ })) 'Cyan'
      $ans = Read-Host 'Press Enter when done (or type a short answer)'
      Set-Content -LiteralPath (Join-Path $gdir 'ANSWER.txt') -Value $(if ($ans) { $ans } else { 'ok' }) -Encoding UTF8
    }
  }.GetNewClosure()
  $run = Invoke-PackGateProcess -ScriptPath (Join-Path $gpDir ([string]$g.script -replace '/', '\')) -CtxFile $ctxFile -ResultFile $resultFile -LogFile (Join-Path $gdir 'gate.out.log') -TimeoutMinutes $timeout -OnTick $onTick
  # A gate that was stopped (timeout) or crashed can leave its lock behind. Remove it only when its owner
  # is a process that is no longer alive and the gate itself wrote it; never a lock of a living process.
  try {
    $lockFile = Join-Path $LockDir 'stelmar-gate.lock'
    if (Test-Path -LiteralPath $lockFile) {
      $lt = @(Get-Content -LiteralPath $lockFile -ErrorAction Stop)
      $lg = @($lt | Where-Object { $_ -match '^gate=' } | Select-Object -First 1)
      $lp = @($lt | Where-Object { $_ -match '^pid=\d+' } | Select-Object -First 1)
      if ($lg.Count -gt 0 -and $lp.Count -gt 0 -and ($lg[0] -split '=', 2)[1] -eq $id -and -not (Get-Process -Id ([int](($lp[0] -split '=')[1])) -ErrorAction SilentlyContinue)) {
        Remove-Item -LiteralPath $lockFile -Force
        Say "removed the stale lock of gate $id (its process had ended)"
      }
    }
  } catch { Say ('could not check the gate lock: ' + $_.Exception.Message) }
  $minutes = [math]::Round(((Get-Date) - $began).TotalMinutes, 1)
  $res = Read-PackJson -Path $resultFile -AsHashtable
  $status = 'INVALID'; $reason = ''
  if ($run.TimedOut) { $status = 'INVALID'; $reason = "stopped after $timeout minutes (too long)" }
  elseif ($null -eq $res) {
    $tail = ''
    $errLog = Join-Path $gdir 'gate.out.log.err'
    if (Test-Path -LiteralPath $errLog) { $tail = (@(Get-Content -LiteralPath $errLog -Tail 3) -join ' | ') }
    $reason = "the gate script ended without a result (exit $($run.ExitCode)). $tail"
  } else { $status = [string]$res['status']; $reason = [string]$res['headline'] }
  $fields = @{ status = $status; ended = (Get-Date).ToString('o'); minutes = $minutes; reason = $reason }
  Set-PackGateStatus -State $state -Id $id -Fields $fields
  if ($status -eq 'PENDING') { [void]$deferred.Add($id); Set-PackGateStatus -State $state -Id $id -Fields @{ attempts = ($attempts - 1) } }
  Save-PackState -WorkDir $WorkDir -State $state
  $colour = switch ($status) { 'PASS' { 'Green' } 'FAIL' { 'Red' } 'INFO' { 'Cyan' } default { 'Yellow' } }
  Write-Host (Format-PackGateLine -Id $id -Status $status -Headline $reason -Rig $(if ($res) { [string]$res['rig'] } else { '' }) -Minutes $minutes) -ForegroundColor $colour
  Add-Content -LiteralPath (Join-Path $WorkDir 'runner.log') -Value (Format-PackGateLine -Id $id -Status $status -Headline $reason -Minutes $minutes) -Encoding UTF8
  # A gate that failed is not run again in the same invocation, even with -Retry.
  if ($status -eq 'FAIL') { [void]$deferred.Add($id) }
}

function Confirm-Restart {
  param([string[]]$For)
  Banner (@('A RESTART IS NEEDED', ('These gates must start within an hour of a restart: ' + ($For -join ', ')), 'Save your work. The pack will carry on by itself when you log in again.', 'The laptop restarts only if you type RESTART below.')) 'Yellow'
  $a = (Read-Host 'Type RESTART to restart now, or press Enter to skip').Trim()
  $a -ceq 'RESTART'
}

function Invoke-RestartNow {
  $extra = @('-Profile', $Profile, '-PackRoot', ('"{0}"' -f $PackRoot), '-WorkDir', ('"{0}"' -f $WorkDir), '-LockDir', ('"{0}"' -f $LockDir), '-BackupRoot', ('"{0}"' -f $BackupRoot))
  if ($Emulate) { $extra += @('-Emulate', ('"{0}"' -f $Emulate)) }
  if ($quickRun) { $extra += '-Quick' }
  if ($Only.Count -gt 0) { $extra += @('-Only', ($Only -join ',')) }
  $how = Register-PackResume -RunScript $runnerScript -ExtraArguments $extra -Admin (Test-PackAdmin)
  $state['restart']['requested_at'] = (Get-Date).ToString('o')
  $state['restart']['boot_time_before'] = (Get-PackBootInfo).BootTime.ToString('o')
  Save-PackState -WorkDir $WorkDir -State $state
  Say "registered: $how"
  Say 'restarting in 15 seconds (shutdown /a cancels)'
  & shutdown.exe /r /t 15 /c 'STELMAR gate pack: restart for the cold-cache gates' | Out-Null
}

# ---- Keep the laptop awake while the pack runs (system and display), until this process ends -----
# SetThreadExecutionState is per thread; this thread runs the whole loop. Nothing is changed in the
# power plan, so there is nothing to put back: the request ends with the process.
try {
  Add-Type -Namespace WbPack -Name Awake -MemberDefinition '[System.Runtime.InteropServices.DllImport("kernel32.dll")] public static extern uint SetThreadExecutionState(uint flags);' -ErrorAction Stop
  # ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ES_DISPLAY_REQUIRED
  [void][WbPack.Awake]::SetThreadExecutionState([uint32]2147483651)
  Say 'the laptop is kept awake while the pack runs (sleep and screen-off are blocked until it ends)'
} catch { Say ('warning: could not block sleep: ' + $_.Exception.Message) }

# ---- Gates the manifest skips under -Quick (a gate named with -Only still runs) ------------------
if ($quickRun) {
  foreach ($g in @($manifest.gates)) {
    $gid = [string]$g.id
    $qs = Get-PackProp $g 'quick' $null
    if (-not ($qs -and (Get-PackProp $qs 'skipped' $false))) { continue }
    if ($Only.Count -gt 0 -and $Only -contains $gid) { continue }
    if ($inPack -notcontains $gid) { continue }
    $gst = [string](Get-PackProp (Get-PackGateState -State $state -Id $gid) 'status' '')
    if (@('PASS', 'FAIL', 'INFO', 'SKIPPED') -contains $gst) { continue }
    Set-PackGateStatus -State $state -Id $gid -Fields @{ status = 'SKIPPED'; reason = ([string](Get-PackProp $qs 'note' 'skipped under -Quick')); ended = (Get-Date).ToString('o') }
    Say ("skipped $gid under -Quick: " + [string](Get-PackProp $qs 'note' 'not part of the dry run'))
  }
  Save-PackState -WorkDir $WorkDir -State $state
}

# ---- The loop -------------------------------------------------------------------------------------
$stopRequested = $false
$restartsAsked = 0
$idleChecks = 0
while (-not $stopRequested) {
  $need = Test-AnyPendingNeed
  $facts = Get-RunnerFacts -PendingNeeds $need
  $up = Get-PackBootInfo
  $skipNow = @($Skip) + @($deferred)
  $sel = Select-PackNextGate -Manifest $manifest -State $state -UptimeMin $up.UptimeMin -Facts $facts -Only $Only -Skip $skipNow -InPack $inPack -Retry:$Retry -Unattended:$Unattended -RestartAllowed:(-not $NoRestart -and -not $Unattended)
  $lastSel = $sel
  if ($sel.Next) { Invoke-OneGate $sel.Next; continue }

  # Nothing can run this minute.
  if ($sel.NeedsRestart -and -not $NoRestart -and -not $Unattended -and $restartsAsked -lt 3) {
    $restartsAsked++
    if (Confirm-Restart -For $sel.RestartFor) { Invoke-RestartNow; exit 0 }
    foreach ($id in $sel.RestartFor) { [void]$deferred.Add($id) }
    continue
  }
  $waiting = @($sel.Waiting)
  if ($waiting.Count -eq 0) { break }
  $onlyUptime = (@($waiting | Where-Object { @($_.Reasons | Where-Object { $_ -notmatch 'laptop up for' }).Count -gt 0 }).Count -eq 0)
  if ($onlyUptime -and @($sel.WaitUptime).Count -gt 0) {
    $left = [math]::Ceiling(($sel.WaitUptime | ForEach-Object { $_.MinutesLeft } | Measure-Object -Minimum).Minimum)
    Say ("waiting {0} minutes for the laptop to settle (a settled gate needs 3 hours of uptime); nothing else is ready" -f $left)
    Start-Sleep -Seconds ([int][math]::Min(300, [math]::Max(30, $left * 60)))
    continue
  }
  # Something is missing that only a person can fix.
  $lines = @('These gates are waiting:')
  foreach ($w in $waiting) { $lines += ('  {0}: {1}' -f $w.Id, ($w.Reasons -join '; ')) }
  if ($Unattended) {
    $idleChecks++
    if ($idleChecks -gt $UnattendedWaitMinutes) { break }
    Say "unattended: waiting 60 seconds for the missing things, then checking again (up to $UnattendedWaitMinutes minutes)"
    Start-Sleep -Seconds 60
    continue
  }
  Banner $lines 'Yellow'
  $a = (Read-Host 'Fix what is listed and press Enter to check again, or type S to stop and save the results').Trim().ToUpper()
  if ($a -eq 'S') { $stopRequested = $true }
  else { $deferred.Clear() }
}

# ---- The end --------------------------------------------------------------------------------------
# What was still waiting, and why, goes into the state so the summary can say it.
if ($lastSel) {
  foreach ($w in @($lastSel.Waiting)) {
    $gs = Get-PackGateState -State $state -Id $w.Id
    $st = [string](Get-PackProp $gs 'status' '')
    if (-not $st -or $st -eq 'PENDING' -or $st -eq 'interrupted') {
      Set-PackGateStatus -State $state -Id $w.Id -Fields @{ status = 'PENDING'; reason = ($w.Reasons -join '; ') }
    }
  }
  Save-PackState -WorkDir $WorkDir -State $state
}
$fp =Read-PackJson -Path (Join-Path $WorkDir 'fingerprint.json')
$commit = ''
$pm = Read-PackJson -Path $packManifestPath
if ($pm) { $commit = [string](Get-PackProp $pm 'source_commit' '') }
$cleanupLines = @()
if (Test-PackResumeRegistered) { $cleanupLines = @(Invoke-PackCleanup -WorkDir $WorkDir -Testbench $testbench) }
else { $cleanupLines = @(Invoke-PackCleanup -WorkDir $WorkDir -Testbench $testbench) }
$stillChanged = @(Get-PackStillChanged -WorkDir $WorkDir)
$summary = New-PackSummary -Manifest $manifest -State $state -Machine $machine -Emulated $(if ($isEmu) { $Profile } else { '' }) -SourceCommit $commit -InPack $inPack -Fingerprint $fp -StillChanged $stillChanged -RunLabel $RunLabel
Write-PackJson -Path (Join-Path $WorkDir 'summary.json') -Value $summary -Depth 14
$zipPath = ''
if (-not $NoZip) {
  $zipDir = $PackRoot
  try { $probe = Join-Path $zipDir '.write-test'; Set-Content -LiteralPath $probe -Value 'x' -ErrorAction Stop; Remove-Item -LiteralPath $probe -Force } catch { $zipDir = $WorkDir }
  $zipPath = Join-Path $zipDir ("gate-results-{0}.zip" -f $machine)
  $z = New-PackZip -WorkDir $WorkDir -ZipPath $zipPath -Summary $summary -CommitText $commit
}
$gates = @($summary['gates'])
$nPass = @($gates | Where-Object { $_['status'] -eq 'PASS' }).Count
$nFail = @($gates | Where-Object { $_['status'] -eq 'FAIL' }).Count
$nInfo = @($gates | Where-Object { $_['status'] -eq 'INFO' }).Count
$nInv = @($gates | Where-Object { $_['status'] -eq 'INVALID' }).Count
$pend = @($summary['pending'])
$endLines = @('STELMAR gate pack: end of this run', ('  PASS {0}   FAIL {1}   INFO {2}   INVALID {3}   still waiting {4}   not in this pack {5}' -f $nPass, $nFail, $nInfo, $nInv, $pend.Count, @($summary['not_in_pack']).Count))
foreach ($g in $gates) { $endLines += ('  ' + (Format-PackGateLine -Id ([string]$g['id']) -Status ([string]$g['status']) -Headline ([string]$g['headline']) -Rig ([string]$g['rig']) -Minutes $g['minutes'])) }
if ($pend.Count -gt 0) { $endLines += 'Still waiting:'; foreach ($p in $pend) { $endLines += ('  {0} {1}: {2}' -f $p['priority'], $p['id'], $p['reason']) } }
foreach ($n in @($summary['not_in_pack'])) { $endLines += ('  {0} {1}: not in this pack' -f $n['priority'], $n['id']) }
if ($zipPath) { $endLines += ''; $endLines += "Results zip: $zipPath"; $endLines += 'Give this zip to the pack owner.' }
if ($stillChanged.Count -gt 0) { $endLines += ''; $endLines += 'STILL CHANGED ON THIS LAPTOP (run run-gates.ps1 -Cleanup, or put back by hand):'; foreach ($s in $stillChanged) { $endLines += ('  ' + $s) } }
else { $endLines += ''; $endLines += 'Nothing is left changed by the pack (logon task removed, volume, mute and power settings put back).' }
Banner $endLines $(if ($nFail -gt 0 -or $stillChanged.Count -gt 0) { 'Red' } elseif ($pend.Count -gt 0) { 'Yellow' } else { 'Green' })
if ($nFail -gt 0) { exit 1 }
exit 0
