<#
.SYNOPSIS
  Phase 3: re-runs the Phase 2 3-WER parity gate (wb-engine against whisper-cli v1.9.5 on identical chunks) after
  the coverage-floor change (owner, 13 Nov 2026; GATES Open items "Engine changes in Phase 3").

.DESCRIPTION
  Same method as 10 Nov 2026: medium f16, the 60 FLEURS clips of eval\clips\phase2-verify-fleurs.txt (20 each for
  en, hi, ur), whisper-cli decodes the exact chunks wb-engine cuts, previous-text context off on both sides,
  the portable AVX2 engine on the CPU, Legion native. Expected: PASS with 60 of 60 identical (FLEURS speech rates
  sit above the new floors hi 4.28 and ur 4.43).

  The harness fingerprint uses the engine's version string, not the binary's hash, so the old parity results must be
  moved away for the runs to happen again: they are moved (not deleted) to -Keep, with the old evidence file.
  The harness waits for the request flag and the gate lock and holds its own busy lock, so this script does not take
  the gate lock. It refuses to start while an overlay tool or a build is running. Nothing else heavy may run.
  Evidence: docs\evidence\phase-2\wer-parity.txt, rewritten by scripts\gates\parity_report.py.
#>
[CmdletBinding()]
param(
  # Run-name prefixes. The gate pair only; add 'verify-cli-', 'verify-wb-' for the information runs (run 1).
  [string[]]$Only = @('verify-clichunks-medium-', 'verify-wbpar-medium-'),
  [string]$Keep = 'D:\STELMAR-Gate\parity-before-floor-change',
  [string]$Engine,
  # Write the evidence again from the results on disk; no decoding and nothing moved.
  [switch]$ReportOnly
)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo = (Resolve-Path (Join-Path $here '..\..')).Path
if (-not $Engine) { $Engine = Join-Path $repo 'src-tauri\binaries\wb-engine-avx2-x86_64-pc-windows-msvc.exe' }
Import-Module (Join-Path $repo 'scripts\lib\PsEnv.psm1') -Force
Set-WbCleanPSModulePath
Set-WbInvariantCulture
Import-Module (Join-Path $repo 'scripts\lib\GateRun.psm1') -Force
$python = Join-Path $repo 'eval\.venv\Scripts\python.exe'
$resultsDir = Join-Path $repo 'eval\results'
$evidence = Join-Path $repo 'docs\evidence\phase-2\wer-parity.txt'
if (-not (Test-Path -LiteralPath $python)) { throw 'the evaluation environment is missing: uv sync --project eval' }
if (-not $ReportOnly) {
  if (-not (Test-Path -LiteralPath $Engine)) { throw "the engine is not built: $Engine (scripts\build-engines.ps1 -Variants avx2)" }
  $overlay = @(Get-WbOverlayTools)
  $builds = @(Get-WbRunningBuilds)
  if ($overlay.Count -or $builds.Count) { throw "refusing to run: overlay tools ($($overlay -join ', ')) or builds ($($builds -join ', ')) are running" }
  New-Item -ItemType Directory -Force -Path $Keep | Out-Null
  $stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
  # Keep what was there: the old results and the old evidence file.
  foreach ($prefix in $Only) {
    foreach ($d in Get-ChildItem -LiteralPath $resultsDir -Directory -Filter "$prefix*" -ErrorAction SilentlyContinue) {
      Move-Item -LiteralPath $d.FullName -Destination (Join-Path $Keep "$($d.Name)-$stamp")
    }
  }
  if (Test-Path -LiteralPath $evidence) { Copy-Item -LiteralPath $evidence -Destination (Join-Path $Keep "wer-parity-$stamp.txt") }
  $env:WB_ENGINE = $Engine
  Remove-Item Env:\WB_ENGINE_DEVICE -ErrorAction SilentlyContinue
  $env:WB_MACHINE = 'Legion'
  $log = Join-Path $Keep "run-$stamp.log"
  $start = Get-Date
  $all = @(); $exit = 0
  foreach ($prefix in $Only) {
    $r = Invoke-WbNative -FilePath $python -ArgumentList @('-X', 'utf8', (Join-Path $repo 'eval\run.py'), 'all', '--only', $prefix)
    $all += $r.Output + $r.Error
    if ($r.ExitCode -ne 0) { $exit = $r.ExitCode }
  }
  $all | Set-Content -LiteralPath $log -Encoding utf8
  Write-Host ("harness exit {0} after {1:N1} min; log {2}" -f $exit, ((Get-Date) - $start).TotalMinutes, $log)
  if ($exit -ne 0) { throw "the harness failed (exit $exit); see $log. The old results are in $Keep" }
}
$rep = Invoke-WbNative -FilePath $python -ArgumentList @('-X', 'utf8', (Join-Path $repo 'scripts\gates\parity_report.py'), '--out', $evidence)
$rep.Output | ForEach-Object { Write-Host $_ }
if ($rep.ExitCode -ne 0) { Write-Host "parity_report.py exit $($rep.ExitCode): $($rep.Error -join ' ')"; exit $rep.ExitCode }
Write-Host "evidence: $evidence"
