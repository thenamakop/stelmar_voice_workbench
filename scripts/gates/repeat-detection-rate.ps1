<#
.SYNOPSIS
  Measures how many spliced-in repeats the repeated_block detector finds in real speech (R3) and
  writes the evidence.

.DESCRIPTION
  Runs crates\wb-recorder\tests\repeat_detection_rate.rs (release build, ignored by default) over
  FLEURS test clips from eval\data (git-ignored; never committed), and writes
  docs\evidence\phase-2b\repeat-detection-rate.txt from its output. Reads files only: nothing is
  recorded or timed, so no lock, overlay check or Track B pause applies.

    powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\gates\repeat-detection-rate.ps1
#>
[CmdletBinding()]
param(
  [string]$OutFile,
  [string]$Machine = 'Legion'
)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo = (Resolve-Path (Join-Path $here '..\..')).Path
Import-Module (Join-Path $repo 'scripts\lib\PsEnv.psm1') -Force
Set-WbCleanPSModulePath
Set-WbInvariantCulture
Import-Module (Join-Path $repo 'scripts\lib\Redact.psm1') -Force
if (-not $OutFile) { $OutFile = Join-Path $repo 'docs\evidence\phase-2b\repeat-detection-rate.txt' }
New-Item -ItemType Directory -Force -Path (Split-Path -Parent $OutFile) | Out-Null

Push-Location $repo
$ErrorActionPreference = 'Continue'  # cargo writes its progress to stderr
try {
  $out = & cargo test -p wb-recorder --release --test repeat_detection_rate -- --ignored --nocapture 2>&1
  $code = $LASTEXITCODE
} finally {
  Pop-Location
  $ErrorActionPreference = 'Stop'
}
$rate = @($out | ForEach-Object { "$_" } | Where-Object { $_ -match '^RATE ' } | ForEach-Object { $_ -replace '^RATE ', '' })
$rev = (& git -C $repo rev-parse --short HEAD) -join ''
$dirty = @(& git -C $repo status --porcelain).Count -gt 0
$lines = @()
$lines += 'repeated_block detection rate on real speech with spliced repeats (R3, Phase 2b)'
$lines += "source: commit $rev$(if ($dirty) { ' with uncommitted or untracked changes' })"
$lines += "machine: $Machine; reads files only"
$lines += "test: crates/wb-recorder/tests/repeat_detection_rate.rs, exit code $code"
$lines += ''
if ($code -ne 0 -or $rate.Count -eq 0) {
  $lines += 'FAIL: the test did not produce a report. Last output:'
  $lines += @($out | ForEach-Object { "  $_" } | Select-Object -Last 30)
} else {
  $lines += $rate
}
Write-WbEvidence -Path $OutFile -Lines $lines -MachineLabel $Machine
Write-Host "$(if ($code -eq 0) { 'DONE' } else { 'FAIL' }): $OutFile"
exit $code
