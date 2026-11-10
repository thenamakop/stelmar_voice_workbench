<#
.SYNOPSIS
  Phase 2 supervisor kill test with the real engine, under an emulation profile.

.DESCRIPTION
  Builds a 5-minute FLEURS English WAV (scratch folder, never committed), then runs the ignored test
  crates\wb-supervisor\tests\kill_real.rs inside `wb-emulate --profile emu-low` (default): the supervisor
  runs `wb-engine batch` (avx2, small model) to the end once without interruption, then again while the
  engine is killed at five random points (seeded). The committed transcript after the kills must equal
  the uninterrupted one: same segment times, same text, no overlap, no repeat.

  Not timing-sensitive (takes no gate lock) but refuses to start while an engine build runs and while
  overlay tools are open, like every gate run. Evidence: docs\evidence\phase-2\supervisor-kill-<profile>.txt.
#>
[CmdletBinding()]
param(
  [string]$Profile = 'emu-low',
  [int]$Threads = 3,
  [string]$Scratch = 'D:\STELMAR-Gate\killtest',
  [string]$OutDir,
  [string]$Python,
  [string]$Emulate
)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo = (Resolve-Path (Join-Path $here '..\..')).Path
if (-not $OutDir) { $OutDir = Join-Path $repo 'docs\evidence\phase-2' }
if (-not $Python) { $Python = Join-Path $repo 'eval\.venv\Scripts\python.exe' }
if (-not $Emulate) { $Emulate = Join-Path $repo 'target\release\wb-emulate.exe' }
Import-Module (Join-Path $repo 'scripts\lib\PsEnv.psm1') -Force
Set-WbCleanPSModulePath
Set-WbInvariantCulture
Import-Module (Join-Path $repo 'scripts\lib\GateRun.psm1') -Force
Import-Module (Join-Path $repo 'scripts\lib\Redact.psm1') -Force

Assert-WbNoOverlayTools
Assert-WbNoBuilds

$engineSrc = Join-Path $repo 'src-tauri\binaries\wb-engine-avx2-x86_64-pc-windows-msvc.exe'
$wav = Join-Path $Scratch 'concat-5min.wav'
$outRun = Join-Path $Scratch "run-$Profile"
New-Item -ItemType Directory -Force -Path $Scratch | Out-Null
if (Test-Path $outRun) { Remove-Item $outRun -Recurse -Force }
$engine = Join-Path $Scratch 'wb-engine-killtest.exe'
Copy-Item $engineSrc $engine -Force

$mk = Invoke-WbNative -FilePath $Python -ArgumentList @((Join-Path $here 'make_concat.py'), '--out', $wav)
if ($mk.ExitCode -ne 0) { throw "make_concat failed: $($mk.Output -join ' ') $($mk.Error -join ' ')" }
$wavLine = $mk.Output | Select-Object -Last 1

# Build the test binary (a build: it must not overlap a timing-sensitive run, and none is running).
$build = Invoke-WbNative -FilePath 'cargo' -ArgumentList @('test', '-p', 'wb-supervisor', '--test', 'kill_real', '--no-run', '--message-format=json')
if ($build.ExitCode -ne 0) { throw "cargo test --no-run failed: $($build.Error -join ' ')" }
$testExe = $null
foreach ($l in $build.Output) { if ($l -match '"executable":"([^"]+kill_real[^"]+\.exe)"') { $testExe = ($Matches[1] -replace '\\\\', '\') } }
if (-not $testExe -or -not (Test-Path $testExe)) { throw 'could not find the kill_real test executable' }

$env:WB_KILLTEST_ENGINE = $engine
$env:WB_KILLTEST_WAV = $wav
$env:WB_KILLTEST_MODEL = Join-Path $repo 'models\ggml-small.bin'
$env:WB_KILLTEST_VAD = Join-Path $repo 'models\ggml-silero-v6.2.0.bin'
$env:WB_KILLTEST_OUT = $outRun
$env:WB_KILLTEST_THREADS = "$Threads"
$started = Get-Date
$argsList = if ($Profile -eq 'native') { @('--ignored', '--nocapture') } else { @('--profile', $Profile, '--log', (Join-Path $Scratch "emulate-$Profile.log"), '--', $testExe, '--ignored', '--nocapture') }
$exe = if ($Profile -eq 'native') { $testExe } else { $Emulate }
$run = Invoke-WbNative -FilePath $exe -ArgumentList $argsList
$seconds = [int]((Get-Date) - $started).TotalSeconds
$report = @()
if (Test-Path (Join-Path $outRun 'report.txt')) { $report = Get-Content (Join-Path $outRun 'report.txt') }

$lines = @(
  "Supervisor kill test with the real engine (Phase 2), profile $Profile",
  "date: $((Get-Date).ToString('yyyy-MM-dd HH:mm', [Globalization.CultureInfo]::InvariantCulture)) (IST on the Legion); ran $seconds s",
  ('overlay tools closed: ' + $(if ((Get-WbOverlayTools).Count -eq 0) { 'yes' } else { 'NO' })),
  'no engine build was running when the test started (checked); the test binary was built before it ran',
  "engine: wb-engine avx2 (copy named wb-engine-killtest.exe, sha256 $((Get-FileHash $engine -Algorithm SHA256).Hash.ToLowerInvariant())), model small f16, $Threads threads, previous-text context off",
  "audio: $wavLine (FLEURS English test utterances in id order joined with 0.8 s of silence; not committed)",
  "test: crates/wb-supervisor/tests/kill_real.rs; kill points drawn with a fixed seed (20261110) among the segments of the uninterrupted run; the engine is killed with taskkill /F while decoding the next chunk",
  "test exit code: $($run.ExitCode)",
  ''
) + $report
if ($run.ExitCode -ne 0 -and $report.Count -eq 0) { $lines += ($run.Output | Select-Object -Last 15); $lines += ($run.Error | Select-Object -Last 15) }
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$path = Join-Path $OutDir "supervisor-kill-$Profile.txt"
Write-WbEvidence -Path $path -Lines $lines
Write-Host "evidence: $path"
$lines | Select-Object -Last 8
