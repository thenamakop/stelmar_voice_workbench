<#
.SYNOPSIS
  Runs a command on the Legion as if on a test laptop (development only; docs/LEGION-FIRST.md).

.DESCRIPTION
  Thin wrapper over crates/wb-emulate: builds it if needed, then runs the command inside a Job
  Object limited to the profile's cores (chosen by efficiency class) and 11 GiB of commit, with
  the NVIDIA GPU hidden from CUDA and only the Intel GPU visible to Vulkan. The job and every
  process in it end when the command ends. wb-emulate is never bundled with the app.

.EXAMPLE
  .\scripts\emulate.ps1 -Profile emu-low -- .\target\release\wb-emulate.exe probe
  .\scripts\emulate.ps1 -Profile emu-mid -Log emu.log -- .\target\gate\stelmar-voice-workbench.exe
#>
param(
  [Parameter(Mandatory = $true)] [ValidateSet('emu-low', 'emu-mid', 'native')] [string]$Profile,
  [string]$Log,
  [Parameter(ValueFromRemainingArguments = $true)] [string[]]$Command
)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$root = Resolve-Path (Join-Path $here '..')
$exe = Join-Path $root 'target\release\wb-emulate.exe'
if (-not (Test-Path $exe)) {
  Push-Location $root
  try { cargo build --release -p wb-emulate; if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' } } finally { Pop-Location }
}
$cmd = @($Command | Where-Object { $_ -ne '--' })
if ($cmd.Count -eq 0) { throw 'Give the command to run after --' }
$argsList = @('--profile', $Profile)
if ($Log) { $argsList += @('--log', $Log) }
$argsList += '--'
$argsList += $cmd
& $exe @argsList
exit $LASTEXITCODE
