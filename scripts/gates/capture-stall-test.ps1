<#
.SYNOPSIS
  Runs the capture-stall test on the real cable (the capture thread kept away for 300 ms, 700 ms
  and longer than the granted device buffer) and writes the evidence.

.DESCRIPTION
  Runs crates\wb-recorder\tests\capture_stall.rs (release build) with WB_CAPTURE_STALL_TEST=1: it
  plays a quiet noise into the virtual cable, records the cable through the real WASAPI stream
  and the real recorder, and stalls the data callback. Writes
  docs\evidence\phase-2b\capture-stall.txt from its output: the device buffer asked for and
  granted, the callback period measured in the app, and for each stall the longest callback gap,
  the margin to the granted buffer, the audio recorded against the wall clock and the xruns.

  It uses the cable, so nothing else may play into it and no gate may run: overlay tools and
  builds are refused here as in the gates. About a minute plus the build.

    powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\gates\capture-stall-test.ps1
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
Import-Module (Join-Path $repo 'scripts\lib\GateRun.psm1') -Force
Import-Module (Join-Path $repo 'scripts\lib\Redact.psm1') -Force
if (-not $OutFile) { $OutFile = Join-Path $repo 'docs\evidence\phase-2b\capture-stall.txt' }
New-Item -ItemType Directory -Force -Path (Split-Path -Parent $OutFile) | Out-Null
Assert-WbNoOverlayTools

Push-Location $repo
$env:WB_CAPTURE_STALL_TEST = '1'
try {
  # Build first, so the test's own timing does not include a compile.
  $build = Invoke-WbNative -FilePath 'cargo' -ArgumentList @('test', '-p', 'wb-recorder', '--release', '--test', 'capture_stall', '--no-run')
  if ($build.ExitCode -ne 0) { throw "The test did not build: $((@($build.Error) + @($build.Output)) -join ' ')" }
  $started = Get-Date
  $run = Invoke-WbNative -FilePath 'cargo' -ArgumentList @('test', '-p', 'wb-recorder', '--release', '--test', 'capture_stall', '--', '--nocapture')
  $code = $run.ExitCode
  $out = @($run.Output) + @($run.Error)
} finally {
  Remove-Item Env:\WB_CAPTURE_STALL_TEST -ErrorAction SilentlyContinue
  Pop-Location
}
$numbers = @($out | ForEach-Object { "$_" } | Where-Object { $_ -match '^(baseline|pause) ?' })
$skipped = @($out | ForEach-Object { "$_" } | Where-Object { $_ -match '^capture_stall: skipped' })
$result = @($out | ForEach-Object { "$_" } | Where-Object { $_ -match '^test result|^test .* \.\.\. (ok|FAILED)' })
$rev = (& git -C $repo rev-parse --short HEAD) -join ''
$dirty = @(& git -C $repo status --porcelain -- . ':!docs/evidence').Count -gt 0
$lines = @()
$lines += 'capture-stall test on the real cable (12 Nov 2026 decision: device buffer 1,000 ms, then 500, 200 ms, then the default; shared mode)'
$lines += "source: commit $rev$(if ($dirty) { ' with uncommitted or untracked changes' })"
$lines += "machine: $Machine; started $($started.ToString('yyyy-MM-dd HH:mm:ss')) (machine clock)"
$lines += 'method: the test (crates/wb-recorder/tests/capture_stall.rs, release build) plays a quiet noise into the cable, opens the cable through capture::open_for_test (the way a session opens it, with the same buffer request and probe), hands the stream to the real recorder, and sleeps inside the data callback once (the stand-in for the capture thread being kept away from the device). It then reads the stop report, session.log and session.json.'
$lines += "exit code: $code"
$lines += ''
$lines += 'what each line says: "baseline" is a recording with no stall (callback period measured in the app; the shortest period is two packets handed over back to back); "pause N ms" is a stall of N ms in the data callback (longest gap between callbacks, the granted device buffer, the margin left, the audio recorded against the wall clock, driver xruns); the last pause is longer than the granted buffer on purpose.'
$lines += ''
if ($numbers.Count -eq 0 -or $skipped.Count -gt 0) {
  $lines += 'FAIL: the test printed no numbers or was skipped (it did not run). Last output:'
  $lines += @($out | ForEach-Object { "  $_" } | Select-Object -Last 30)
} else {
  $lines += $numbers
}
$lines += ''
$lines += $result
$lines += $(if ($code -eq 0 -and $numbers.Count -gt 0 -and $skipped.Count -eq 0) { 'verdict: PASS (every assertion held: no xrun, no overrun, no dropped sample and no position break for 300 ms and 700 ms; the stall beyond the granted buffer was an xrun, in session.log and session.json, with the missing audio and a negative margin)' } else { 'verdict: FAIL' })
Write-WbEvidence -Path $OutFile -Lines $lines -MachineLabel $Machine
Write-Host "$(if ($code -eq 0) { 'DONE' } else { 'FAIL' }): $OutFile"
exit $code
