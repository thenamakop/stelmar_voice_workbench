<#
.SYNOPSIS
  Phase 3 night run: the system-check gates, then the parity re-run, one after the other on the quiet Legion.

.DESCRIPTION
  1. scripts\gates\system-check-gate.ps1   quick under emu-low, emu-mid, native; full under emu-low; fingerprint hook
  2. scripts\gates\parity-rerun.ps1        3-WER parity re-run after the coverage-floor change

  Before starting: close RTSS, MSI Afterburner, LatencyMon, browsers playing video and every build or test run
  (no cargo, pnpm or engine build); the gate build, the engines and the wb-emulate release build must exist
  (scripts\gates\build-gate.ps1, scripts\build-engines.ps1). Expected time: about 1.5 hours.
  Each step writes its own evidence file under docs\evidence\; a failed step does not stop the next one, and
  the exit code is 1 if any step failed.
#>
[CmdletBinding()]
param([switch]$SkipParity, [switch]$SkipSystemCheck)
$ErrorActionPreference = 'Continue'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$bad = 0
if (-not $SkipSystemCheck) {
  & (Join-Path $here 'system-check-gate.ps1')
  if ($LASTEXITCODE -ne 0) { $bad++; Write-Host 'system-check-gate.ps1 reported a problem' }
}
if (-not $SkipParity) {
  & (Join-Path $here 'parity-rerun.ps1')
  if ($LASTEXITCODE -ne 0) { $bad++; Write-Host 'parity-rerun.ps1 reported a problem' }
}
exit $(if ($bad) { 1 } else { 0 })
