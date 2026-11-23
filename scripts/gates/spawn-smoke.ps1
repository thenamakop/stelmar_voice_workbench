<#
.SYNOPSIS
  Phase 4 smoke of the engine spawn path under an emulation profile (not a timing gate).

.DESCRIPTION
  The supervisor's spawn path changed after the Phase 3 evidence build (6253468): every engine now starts
  CREATE_SUSPENDED, is put in a Job Object of its own (with an optional per-process memory limit) and only then
  resumed, and a release-config start also drops GGML_VK_VISIBLE_DEVICES and CUDA_VISIBLE_DEVICES. This script
  checks, inside `wb-emulate --profile <profile>` (so each engine job nests inside the emulation job), that:
    1. the 12 restart tests (mock engine) and the memory-limit tests still pass there;
    2. a real `wb-engine bench` on the 60 s Hindi clip, started through EngineProcess::spawn_limited, runs
       without a limit and then with the limit the queue computes from the first run's peak, and the second
       run's peak is at most 70% of its limit; the peak is printed against the limit;
    3. (unless -SkipRealKill) the real-engine split-chunk kill test of Phase 2 (kill_real.rs case 2: three kills
       inside the second half of a split chunk) still gives a transcript identical to the uninterrupted run.
  Takes the gate lock (quiet machine, overlay tools closed, no build running). Evidence:
  docs\evidence\phase-4\spawn-smoke-<profile>.txt, written by this script.
#>
[CmdletBinding()]
param(
  [string]$Profile = 'emu-low',
  [int]$Threads = 3,
  [switch]$SkipRealKill,
  [string]$Scratch = 'D:\STELMAR-Gate\spawn-smoke',
  [string]$OutDir,
  [string]$Python,
  [string]$Emulate,
  [string]$Clip = 'D:\STELMAR-Gate\speed\clip-hi.wav'
)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo = (Resolve-Path (Join-Path $here '..\..')).Path
if (-not $OutDir) { $OutDir = Join-Path $repo 'docs\evidence\phase-4' }
if (-not $Python) { $Python = Join-Path $repo 'eval\.venv\Scripts\python.exe' }
if (-not $Emulate) { $Emulate = Join-Path $repo 'target\release\wb-emulate.exe' }
Import-Module (Join-Path $repo 'scripts\lib\PsEnv.psm1') -Force
Set-WbCleanPSModulePath
Set-WbInvariantCulture
Import-Module (Join-Path $repo 'scripts\lib\GateRun.psm1') -Force
Import-Module (Join-Path $repo 'scripts\lib\Redact.psm1') -Force

foreach ($f in @($Emulate, $Clip)) { if (-not (Test-Path $f)) { throw "Missing $f" } }
New-Item -ItemType Directory -Force -Path $Scratch | Out-Null

# Build every test binary first (builds must not overlap the lock).
Assert-WbNoBuilds
$exes = @{}
foreach ($t in 'restart', 'memory_limit', 'spawn_smoke', 'kill_real') {
  $b = Invoke-WbNative -FilePath 'cargo' -ArgumentList @('test', '-p', 'wb-supervisor', '--test', $t, '--no-run', '--message-format=json')
  if ($b.ExitCode -ne 0) { throw "cargo test --no-run ($t) failed: $($b.Error -join ' ')" }
  foreach ($l in $b.Output) { if ($l -match ('"executable":"([^"]+\\' + $t + '-[^"]+\.exe)"')) { $exes[$t] = ($Matches[1] -replace '\\\\', '\') } }
  if (-not $exes[$t] -or -not (Test-Path $exes[$t])) { throw "could not find the $t test executable" }
}
$engineSrc = Join-Path $repo 'src-tauri\binaries\wb-engine-avx2-x86_64-pc-windows-msvc.exe'
$engineBench = Join-Path $Scratch 'wb-engine-smoke.exe'
$engineKill = Join-Path $Scratch 'wb-engine-killtest.exe'
Copy-Item $engineSrc $engineBench -Force
Copy-Item $engineSrc $engineKill -Force
$vad = Join-Path $repo 'models\ggml-silero-v6.2.0.bin'
$small = Join-Path $repo 'models\ggml-small.bin'
$smokeOut = Join-Path $Scratch "bench-$Profile"
if (Test-Path $smokeOut) { Remove-Item $smokeOut -Recurse -Force }
$wavShort = Join-Path $Scratch 'concat-150s.wav'
$killOut = Join-Path $Scratch "run-split-$Profile"
if (Test-Path $killOut) { Remove-Item $killOut -Recurse -Force }
$wavLine = ''
if (-not $SkipRealKill) {
  $mk = Invoke-WbNative -FilePath $Python -ArgumentList @((Join-Path $here 'make_concat.py'), '--out', $wavShort, '--seconds', '150')
  if ($mk.ExitCode -ne 0) { throw "make_concat failed: $($mk.Output -join ' ') $($mk.Error -join ' ')" }
  $wavLine = $mk.Output | Select-Object -Last 1
}

function Invoke-InProfile([string]$Exe, [string[]]$TestArgs) {
  $a = if ($Profile -eq 'native') { $TestArgs } else { @('--profile', $Profile, '--log', (Join-Path $Scratch "emulate-$Profile.log"), '--', $Exe) + $TestArgs }
  $f = if ($Profile -eq 'native') { $Exe } else { $Emulate }
  $t0 = Get-Date
  $r = Invoke-WbNative -FilePath $f -ArgumentList $a
  [pscustomobject]@{ Run = $r; Seconds = [int]((Get-Date) - $t0).TotalSeconds }
}
function Summary([object]$r) { @($r.Run.Output + $r.Run.Error | Where-Object { $_ -match '^test result|^test .* (ok|FAILED)$|panicked' }) }

$results = [ordered]@{}
Invoke-WbGate -Name "spawn smoke $Profile" -Body {
  $freeMb = [int]((Get-WbFreeRamGB) * 1024)
  $results.restart = Invoke-InProfile $exes.restart @('--test-threads', '1')
  $results.memory = Invoke-InProfile $exes.memory_limit @('--test-threads', '1')
  $env:WB_SMOKE_ENGINE = $engineBench; $env:WB_SMOKE_MODEL = $small; $env:WB_SMOKE_VAD = $vad
  $env:WB_SMOKE_CLIP = $Clip; $env:WB_SMOKE_OUT = $smokeOut; $env:WB_SMOKE_THREADS = "$Threads"
  $env:WB_SMOKE_LANG = 'hi'; $env:WB_SMOKE_AVAILABLE_MB = "$([math]::Min($freeMb, 11264))"
  $results.bench = Invoke-InProfile $exes.spawn_smoke @('--ignored', '--nocapture')
  if (-not $SkipRealKill) {
    $env:WB_KILLTEST_ENGINE = $engineKill; $env:WB_KILLTEST_WAV = $wavShort; $env:WB_KILLTEST_MODEL = $small
    $env:WB_KILLTEST_VAD = $vad; $env:WB_KILLTEST_OUT = $killOut; $env:WB_KILLTEST_THREADS = "$Threads"
    $results.kill = Invoke-InProfile $exes.kill_real @('kills_inside_the_second_half_of_a_split_chunk_lose_and_repeat_no_half', '--exact', '--ignored', '--nocapture')
  }
}

$lines = @("Engine spawn smoke (Phase 4), profile $Profile") + (Get-WbGateHeader -Profile $Profile -Builds @($engineSrc))
$lines += @('', 'Why: the spawn path (suspended start, a Job Object per engine, optional memory limit, device variables dropped in release config) changed after the Phase 3 evidence build 6253468.')
$lines += @('', "== 12 restart tests, mock engine, inside the emulation job ($($results.restart.Seconds) s, exit $($results.restart.Run.ExitCode)) ==") + (Summary $results.restart)
$lines += @('', "== memory-limit tests (mock engine allocates over and under its limit, two engines at once) ($($results.memory.Seconds) s, exit $($results.memory.Run.ExitCode)) ==") + (Summary $results.memory)
$rp = Join-Path $smokeOut 'report.txt'
$lines += @('', "== real wb-engine bench, Hindi 60 s clip, avx2, small f16, $Threads threads, started through the supervisor ($($results.bench.Seconds) s, exit $($results.bench.Run.ExitCode)) ==",
  "engine: wb-engine avx2 (copy wb-engine-smoke.exe, sha256 $((Get-FileHash $engineBench -Algorithm SHA256).Hash.ToLowerInvariant())); clip: speed\clip-hi.wav (FLEURS Hindi, 60 s, not committed)")
if (Test-Path $rp) { $lines += (Get-Content $rp) } else { $lines += ($results.bench.Run.Output + $results.bench.Run.Error | Select-Object -Last 20) }
$lines += (Summary $results.bench)
if ($results.kill) {
  $kp = Join-Path $killOut 'split\report.txt'
  $lines += @('', "== real-engine split-chunk kill test (Phase 2 case 2, three kills inside the second half of a split chunk) ($($results.kill.Seconds) s, exit $($results.kill.Run.ExitCode)) ==", "audio: $wavLine")
  if (Test-Path $kp) { $lines += (Get-Content $kp) } else { $lines += ($results.kill.Run.Output + $results.kill.Run.Error | Select-Object -Last 20) }
  $lines += (Summary $results.kill)
}
$ok = ($results.restart.Run.ExitCode -eq 0) -and ($results.memory.Run.ExitCode -eq 0) -and ($results.bench.Run.ExitCode -eq 0) -and ((-not $results.kill) -or ($results.kill.Run.ExitCode -eq 0))
$lines += @('', "verdict: $(if ($ok) { 'PASS' } else { 'FAIL' })")
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$path = Join-Path $OutDir "spawn-smoke-$Profile.txt"
Write-WbEvidence -Path $path -Lines $lines
Write-Host "evidence: $path"
$lines | Select-Object -Last 40
if (-not $ok) { exit 1 }
