<#
  PackGate.psm1: the helpers a gate script of the pack uses (scripts/gate-pack/gates/<id>.ps1).

  A gate script is started by run-gates.ps1 as
      powershell -File gates\<id>.ps1 -CtxFile <ctx.json> -ResultFile <result.json>
  Every gate script starts with the same four lines:
      $lib = Join-Path $PSScriptRoot '..\..\lib'
      Import-Module (Join-Path $lib 'PackCore.psm1') -Force; Import-Module (Join-Path $lib 'PackGate.psm1') -Force
      $ctx = Read-PackCtx -CtxFile $CtxFile; Initialize-PackGate -Ctx $ctx
  then does its work and writes one result file (Complete-PackGate). A gate that crashes leaves no
  result file; the runner then records INVALID.

  The context (ctx.json) carries: pack_root, work_dir, gate_id, gate_dir, profile ('laptop', or
  'emu-low' / 'emu-mid' for the dry run on the Legion), emulate (path of wb-emulate, dry run only),
  role, quick, machine, lock_dir, min_free_ram_gb, uptime_min, boot_time, admin, rig_prefer,
  mirror_root, backup_root, unattended.

  Windows PowerShell 5.1, ASCII only.
#>

Import-Module (Join-Path $PSScriptRoot 'PackCore.psm1')
Import-Module (Join-Path $PSScriptRoot 'PackNative.psm1')

function Read-PackCtx {
  param([Parameter(Mandatory = $true)] [string]$CtxFile)
  Get-Content -LiteralPath $CtxFile -Raw -Encoding UTF8 | ConvertFrom-Json
}

# Paths inside the pack, in the layout the older gate scripts expect (they resolve ..\.. to the root).
function Get-PackPaths {
  param([Parameter(Mandatory = $true)] $Ctx)
  $r = [string]$Ctx.pack_root
  [pscustomobject]@{
    Root = $r
    Scripts = (Join-Path $r 'scripts')
    Gates = (Join-Path $r 'scripts\gates')
    Lib = (Join-Path $r 'scripts\lib')
    GatePackGates = (Join-Path $r 'scripts\gate-pack\gates')
    Testbench = (Join-Path $r 'target\release\wb-testbench.exe')
    GateExe = (Join-Path $r 'target\gate\stelmar-voice-workbench.exe')
    ReleaseExe = (Join-Path $r 'target\release\stelmar-voice-workbench.exe')
    Engines = (Join-Path $r 'src-tauri\binaries')
    Models = (Join-Path $r 'models')
    BenchClips = (Join-Path $r 'resources\bench')
    Data = (Join-Path $r 'data')
    Emulate = $(if ($Ctx.PSObject.Properties['emulate'] -and $Ctx.emulate) { [string]$Ctx.emulate } else { '' })
  }
}

# Sets the environment the older scripts read, and imports what a gate needs. Call first.
function Initialize-PackGate {
  param([Parameter(Mandatory = $true)] $Ctx)
  $env:WB_PACK_ROOT = [string]$Ctx.pack_root
  $env:WB_PACK_LOCK_DIR = [string]$Ctx.lock_dir
  $env:WB_PACK_MIN_FREE_RAM_GB = [string]$Ctx.min_free_ram_gb
  New-Item -ItemType Directory -Force -Path ([string]$Ctx.gate_dir) | Out-Null
  $lib = Join-Path ([string]$Ctx.pack_root) 'scripts\lib'
  Import-Module (Join-Path $lib 'PsEnv.psm1') -Force
  Set-WbCleanPSModulePath
  Set-WbInvariantCulture
}

function Write-PackGateLog {
  param([Parameter(Mandatory = $true)] $Ctx, [string]$Message)
  $line = '{0} {1}' -f (Get-Date -Format 'HH:mm:ss'), $Message
  Write-Host $line
  try { Add-Content -LiteralPath (Join-Path ([string]$Ctx.gate_dir) 'gate.log') -Value $line -Encoding UTF8 } catch { }
}

# Asks the person something through the runner's console. The gate writes PROMPT.txt in its folder and
# waits for ANSWER.txt, which the runner writes after the person has pressed Enter (or typed text).
# Returns the answer text; throws after -TimeoutMinutes without one. Unattended runs never get here.
function Request-PackPrompt {
  param([Parameter(Mandatory = $true)] $Ctx, [Parameter(Mandatory = $true)] [string]$Message, [int]$TimeoutMinutes = 90)
  $dir = [string]$Ctx.gate_dir
  $ans = Join-Path $dir 'ANSWER.txt'
  if (Test-Path -LiteralPath $ans) { Remove-Item -LiteralPath $ans -Force }
  Set-Content -LiteralPath (Join-Path $dir 'PROMPT.txt') -Value $Message -Encoding UTF8
  $until = (Get-Date).AddMinutes($TimeoutMinutes)
  while (-not (Test-Path -LiteralPath $ans)) {
    if ((Get-Date) -gt $until) { throw 'nobody answered the prompt in time' }
    Start-Sleep -Seconds 2
  }
  $t = (Get-Content -LiteralPath $ans -Raw -Encoding UTF8).Trim()
  Remove-Item -LiteralPath $ans -Force -ErrorAction SilentlyContinue
  $t
}

# Writes the gate's result file. Use New-PackResult for the object.
function Complete-PackGate {
  param([Parameter(Mandatory = $true)] $Ctx, [Parameter(Mandatory = $true)] $Result, [Parameter(Mandatory = $true)] [string]$ResultFile)
  Write-PackResult -Path $ResultFile -Result $Result
  Write-PackGateLog -Ctx $Ctx -Message ('{0}: {1} {2}' -f $Result['id'], $Result['status'], $Result['headline'])
}

# Pure. What a thrown message means for the gate: PENDING (the person can fix it: rig, overlay tools,
# free memory; the gate is not counted as tried) or INVALID (a fault in the run).
function Get-PackErrorStatus {
  param([string]$Message)
  if ($Message -match '^(RIG REFUSED|RIG SELF-TEST FAILED|Overlay tools are running|Only [0-9.,]+ GB of RAM free|An engine build is running)') { return 'PENDING' }
  'INVALID'
}

# Runs a gate's body and always writes a result: what the body returns, or INVALID / PENDING with the
# error text when it throws.
function Invoke-PackGateBody {
  param([Parameter(Mandatory = $true)] $Ctx, [Parameter(Mandatory = $true)] [string]$Id, [Parameter(Mandatory = $true)] [string]$ResultFile, [Parameter(Mandatory = $true)] [scriptblock]$Body)
  try {
    $r = & $Body
    if ($r -is [array]) { $r = $r[-1] }
  } catch {
    $m = $_.Exception.Message
    $r = New-PackResult -Id $Id -Status (Get-PackErrorStatus $m) -Headline $m -Notes @([string]$_.ScriptStackTrace) -Quick ([bool]$Ctx.quick)
  }
  Complete-PackGate -Ctx $Ctx -Result $r -ResultFile $ResultFile
}

# Runs one of the older gate scripts (scripts\gates\*.ps1, with -Profile laptop or an emulation
# profile) in its own Windows PowerShell 5.1 process, output to a log. Returns ExitCode, Log and the
# path of the evidence file it wrote (-EvidenceName under -OutDir).
function Invoke-PackLegacyGate {
  param(
    [Parameter(Mandatory = $true)] $Ctx,
    [Parameter(Mandatory = $true)] [string]$Script,
    [string[]]$Arguments = @(),
    [string]$EvidenceName = '',
    [int]$TimeoutMinutes = 240
  )
  $paths = Get-PackPaths -Ctx $Ctx
  $scriptPath = Join-Path $paths.Gates $Script
  if (-not (Test-Path -LiteralPath $scriptPath)) { throw "the gate script $Script is not in this pack" }
  $outDir = Join-Path ([string]$Ctx.gate_dir) 'evidence'
  New-Item -ItemType Directory -Force -Path $outDir | Out-Null
  $log = Join-Path ([string]$Ctx.gate_dir) ($Script + '.out.log')
  $ps = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
  $argList = @('-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', (ConvertTo-PackArg $scriptPath)) + $Arguments
  $p = Start-Process -FilePath $ps -ArgumentList $argList -PassThru -WindowStyle Hidden -RedirectStandardOutput $log -RedirectStandardError ($log + '.err')
  $null = $p.Handle
  if (-not $p.WaitForExit($TimeoutMinutes * 60000)) {
    $null = Invoke-PackNative -FilePath 'taskkill.exe' -ArgumentList @('/F', '/T', '/PID', "$($p.Id)") -TimeoutSeconds 30
    return [pscustomobject]@{ ExitCode = -1; TimedOut = $true; Log = $log; OutDir = $outDir; Evidence = '' }
  }
  $p.WaitForExit()
  $ev = ''
  if ($EvidenceName) { $c = Join-Path $outDir $EvidenceName; if (Test-Path -LiteralPath $c) { $ev = $c } }
  [pscustomobject]@{ ExitCode = [int]$p.ExitCode; TimedOut = $false; Log = $log; OutDir = $outDir; Evidence = $ev }
}

# The arguments every legacy gate script of the pack gets for the profile and folders of this run.
function Get-PackLegacyCommon {
  param([Parameter(Mandatory = $true)] $Ctx)
  $paths = Get-PackPaths -Ctx $Ctx
  $a = @('-Profile', (ConvertTo-PackArg ([string]$Ctx.profile)), '-Machine', (ConvertTo-PackArg ([string]$Ctx.machine)),
    '-WorkRoot', (ConvertTo-PackArg (Join-Path ([string]$Ctx.work_dir) 'audio')),
    '-BackupRoot', (ConvertTo-PackArg ([string]$Ctx.backup_root)),
    '-OutDir', (ConvertTo-PackArg (Join-Path ([string]$Ctx.gate_dir) 'evidence')),
    '-Exe', (ConvertTo-PackArg $paths.GateExe), '-Testbench', (ConvertTo-PackArg $paths.Testbench))
  if ($paths.Emulate) { $a += @('-Emulate', (ConvertTo-PackArg $paths.Emulate)) }
  $a
}

Export-ModuleMember -Function Read-PackCtx, Get-PackPaths, Initialize-PackGate, Write-PackGateLog, Request-PackPrompt, Complete-PackGate, Get-PackErrorStatus, Invoke-PackGateBody, Invoke-PackLegacyGate, Get-PackLegacyCommon
