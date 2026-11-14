<#
.SYNOPSIS
  Phase 3 informational comparison (owner, 13 Nov 2026): large-v3-turbo q5_0, stock medium, large-v3 q5_0 and
  small, through wb-engine batch with the vulkan engine on the RTX, rolling settings. Quality only; not a gate.

.DESCRIPTION
  Sets: the 20 committed FLEURS clips for en, hi and ur (eval\clips\phase2-verify-fleurs.txt) and 100 Kathbath
  test-unknown utterances for hi and ur (eval\clips\phase3-kathbath-hi.txt, -ur.txt; fixed seed, written by
  scripts\gates\kathbath_clip_list.py). Both models run on the same device (the NVIDIA GPU of the vulkan engine,
  found from `wb-engine info`); speeds for decisions come from the Phase 2 speed tables.
  The evaluation harness waits for the gate request flag and the gate lock and holds its own busy lock, so this
  script does not take the gate lock (it would block the harness). It refuses to start while an overlay tool or a
  build is running. Nothing else heavy may run meanwhile. About 15 minutes on the RTX.
  Evidence: docs\evidence\phase-3\turbo-medium-comparison.txt (written by this script from the harness results).
#>
[CmdletBinding()]
param(
  [string]$Device,                       # 'gpu:N'; default: the NVIDIA GPU listed by `info`
  [string]$Out,
  # Run-name prefixes to run (eval/runs.toml). The report always reads every phase3-tm-* result.
  [string[]]$Only = @('phase3-tm-'),
  # Write the report again from the results on disk; no decoding. -HarnessMinutes is the time of the run that made them.
  [switch]$ReportOnly,
  [double]$HarnessMinutes = 0,
  [string]$Work = 'D:\STELMAR-Gate\turbo-medium'
)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo = (Resolve-Path (Join-Path $here '..\..')).Path
if (-not $Out) { $Out = Join-Path $repo 'docs\evidence\phase-3\turbo-medium-comparison.txt' }
Import-Module (Join-Path $repo 'scripts\lib\PsEnv.psm1') -Force
Set-WbCleanPSModulePath
Set-WbInvariantCulture
Import-Module (Join-Path $repo 'scripts\lib\GateRun.psm1') -Force
Import-Module (Join-Path $repo 'scripts\lib\Redact.psm1') -Force
New-Item -ItemType Directory -Force -Path $Work, (Split-Path -Parent $Out) | Out-Null

$overlay = @(Get-WbOverlayTools)
$builds = @(Get-WbRunningBuilds)
if ($overlay.Count -or $builds.Count) { throw "refusing to run: overlay tools ($($overlay -join ', ')) or builds ($($builds -join ', ')) are running" }

$engine = Join-Path $repo 'src-tauri\binaries\wb-engine-vulkan-x86_64-pc-windows-msvc.exe'
if (-not (Test-Path -LiteralPath $engine)) { throw "the vulkan engine is not built: $engine (scripts\build-engines.ps1 -Variants vulkan)" }
$python = Join-Path $repo 'eval\.venv\Scripts\python.exe'
if (-not (Test-Path -LiteralPath $python)) { throw "the evaluation environment is missing: uv sync --project eval" }
foreach ($f in 'eval\clips\phase3-kathbath-hi.txt', 'eval\clips\phase3-kathbath-ur.txt', 'eval\clips\phase2-verify-fleurs.txt') {
  if (-not (Test-Path -LiteralPath (Join-Path $repo $f))) { throw "missing clip list $f" }
}

# The device: the NVIDIA GPU in the vulkan engine's own list.
$deviceNote = ''
if (-not $Device) {
  $info = Invoke-WbNative -FilePath $engine -ArgumentList @('info')
  $report = $null
  foreach ($l in $info.Output) { if ($l.StartsWith('{')) { $j = $l | ConvertFrom-Json; if ($j.event -eq 'info') { $report = $j } } }
  if (-not $report) { throw 'wb-engine info gave no report' }
  $gpu = @($report.devices | Where-Object { $_.kind -eq 'gpu' -and $_.description -match 'NVIDIA' })
  if ($gpu.Count -ne 1) { throw "expected exactly one NVIDIA GPU in the vulkan engine's list, found $($gpu.Count)" }
  $Device = "gpu:$($gpu[0].index)"
  $deviceNote = "($($gpu[0].description), Vulkan, engine device index $($gpu[0].index))"
}

$env:WB_ENGINE = $engine
$env:WB_ENGINE_DEVICE = $Device
$env:WB_MACHINE = 'Legion'
$log = Join-Path $Work ("run-{0}.log" -f (Get-Date -Format 'yyyyMMdd-HHmmss'))
Write-Host "device $Device $deviceNote; harness log $log"
$start = Get-Date
# Each result folder records the hash of the engine that made it. A result written before the engine file was
# rebuilt (the Urdu coverage floor changed on 13 Nov 2026) was made by the engine whose hash was saved before the
# rebuild; a result written after it by the current engine.
$engineSha = (Get-FileHash -LiteralPath $engine -Algorithm SHA256).Hash.ToLowerInvariant()
$engineBuilt = (Get-Item -LiteralPath $engine).LastWriteTime
$beforeFile = Join-Path $Work 'vulkan-sha-before-floor-change.txt'
$before = if (Test-Path -LiteralPath $beforeFile) { (Get-Content -LiteralPath $beforeFile -Raw).Trim().ToLowerInvariant() } else { $null }
$resultsDir = Join-Path $repo 'eval\results'
function Set-EngineShas {
  foreach ($d in Get-ChildItem -LiteralPath $resultsDir -Directory -Filter 'phase3-tm-*' -ErrorAction SilentlyContinue) {
    $summary = Join-Path $d.FullName 'summary.json'
    if (-not (Test-Path -LiteralPath $summary)) { continue }
    $sha = if ($before -and (Get-Item -LiteralPath $summary).LastWriteTime -lt $engineBuilt) { $before } else { $engineSha }
    Set-Content -LiteralPath (Join-Path $d.FullName 'engine-sha256.txt') -Value $sha -Encoding ascii
  }
}
if ($ReportOnly) {
  Set-EngineShas
  $exit = 0
} else {
  $allOut = @(); $exit = 0
  foreach ($prefix in $Only) {
    $r = Invoke-WbNative -FilePath $python -ArgumentList @('-X', 'utf8', (Join-Path $repo 'eval\run.py'), 'all', '--only', $prefix)
    $allOut += $r.Output + $r.Error
    if ($r.ExitCode -ne 0) { $exit = $r.ExitCode }
  }
  $allOut | Set-Content -LiteralPath $log -Encoding utf8
  Set-EngineShas
}
$run = [pscustomobject]@{ ExitCode = $exit }
Write-Host ("harness exit {0} after {1:N1} min" -f $run.ExitCode, ((Get-Date) - $start).TotalMinutes)

$tmp = Join-Path $Work 'report.txt'
$rep = Invoke-WbNative -FilePath $python -ArgumentList @('-X', 'utf8', (Join-Path $repo 'scripts\gates\turbo_medium_report.py'), $tmp, '--device-note', $deviceNote)
if (-not (Test-Path -LiteralPath $tmp)) { throw "the report was not written: $($rep.Error -join ' ')" }
$lines = @([IO.File]::ReadAllLines($tmp, [Text.Encoding]::UTF8))
$header = @(
  $(if ($ReportOnly) { "date: $(Get-Date -Format 'yyyy-MM-dd HH:mm') (IST on the Legion); report written again from the results on disk (no decoding); the run that made them took $HarnessMinutes min" } else { "date: $(Get-Date -Format 'yyyy-MM-dd HH:mm') (IST on the Legion); harness run time $([math]::Round(((Get-Date) - $start).TotalMinutes, 1)) min; harness exit code $($run.ExitCode)" }),
  "command: powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\gates\turbo-medium-comparison.ps1",
  "machine: Legion (native, no emulation); WB_ENGINE_DEVICE=$Device"
)
Write-WbEvidence -Path $Out -Lines ($header + $lines)
Write-Host "evidence: $Out"
if ($run.ExitCode -ne 0) { exit $run.ExitCode }
