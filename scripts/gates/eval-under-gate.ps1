<#
.SYNOPSIS
  Runs named evaluation runs (eval/run.py all --only <name>) on the Legion GPU under the gate lock, after the quiet-machine
  check (owner, 26 Nov 2026: every decode takes the gate lock after the idle check, even when nobody else holds a lock).

.DESCRIPTION
  Wait-WbQuiet (host CPU under 10%, no busy development processes, none of the owner's programs) runs first and writes
  idle-check.json; then Invoke-WbGate takes D:\Projects\stelmar-gate.lock for the decodes. The evaluation harness waits on
  the gate lock itself (wbeval.common), so it is started with WB_GATE_LOCK pointing at a file that never exists: this script
  holds the real lock. Each run's numbers go to eval/results/<name>/ as usual.

  scripts\gates\eval-under-gate.ps1 -Run tune-small-detect-f16 [-Run ...] [-Device gpu:0]
#>
[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)] [string[]]$Run,
  [string]$Device = 'gpu:0',
  [string]$Engine = 'src-tauri\binaries\wb-engine-cuda-x86_64-pc-windows-msvc.exe',
  [string]$LogDir = 'D:\STELMAR-Gate\eval-gate'
)
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
Import-Module (Join-Path $repo 'scripts\lib\PsEnv.psm1') -Force
Import-Module (Join-Path $repo 'scripts\lib\GateRun.psm1') -Force
Import-Module (Join-Path $repo 'scripts\lib\Quiet.psm1') -Force

New-Item -ItemType Directory -Force -Path $LogDir | Out-Null
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$idleFile = Join-Path $LogDir "idle-check-$stamp.json"
$py = Join-Path $repo 'eval\.venv\Scripts\python.exe'
$exe = (Resolve-Path (Join-Path $repo $Engine)).Path

$quiet = Wait-WbQuiet -Out $idleFile
Write-Host "$(Get-Date -Format 'HH:mm:ss') quiet-machine rule: $quiet (details: $idleFile)"

Invoke-WbGate -Name ("eval " + ($Run -join ', ')) -Body {
  Push-Location $repo
  try {
    $env:WB_ENGINE = $exe
    $env:WB_ENGINE_DEVICE = $Device
    $env:WB_MACHINE = 'Legion'
    $env:WB_GATE_LOCK = Join-Path $LogDir 'never-exists.lock'
    foreach ($name in $Run) {
      Write-Host "$(Get-Date -Format 'HH:mm:ss') decode: $name"
      & $py -X utf8 eval\run.py all --only $name
      if ($LASTEXITCODE -ne 0) { throw "eval/run.py failed for $name (exit $LASTEXITCODE)" }
    }
  } finally {
    Remove-Item Env:\WB_ENGINE, Env:\WB_ENGINE_DEVICE, Env:\WB_MACHINE, Env:\WB_GATE_LOCK -ErrorAction SilentlyContinue
    Pop-Location
  }
}
Write-Host "$(Get-Date -Format 'HH:mm:ss') done: $($Run -join ', ')"
