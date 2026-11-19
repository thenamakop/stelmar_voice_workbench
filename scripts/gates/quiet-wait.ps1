<#
.SYNOPSIS
  Waits for the quiet-machine rule (scripts\lib\Quiet.psm1) and writes the attempts to -Out. Exit 0 when the machine is
  quiet (or the 15% fallback applies), 1 when the rule refuses (Steam or the wallpaper renderer running for 30 minutes,
  or 60 minutes without a quiet machine). Used before gates that do not hold the check themselves (the full system
  check, the speed recheck).
#>
param([string]$Out = 'D:\STELMAR-Gate\idle-check.json')
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
Import-Module (Join-Path $here '..\lib\Quiet.psm1') -Force
Write-Host "display drivers: $(Get-WbGpuDrivers)"
try { $v = Wait-WbQuiet -Out $Out; Write-Host "quiet-machine rule: $v"; exit 0 } catch { Write-Host "REFUSED: $($_.Exception.Message)"; exit 1 }
