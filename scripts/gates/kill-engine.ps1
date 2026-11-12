<#
.SYNOPSIS
  Phase 2 supervisor kill tests with the real engine, under an emulation profile.

.DESCRIPTION
  Builds a 5-minute FLEURS English WAV (scratch folder, never committed), then runs the ignored tests of
  crates\wb-supervisor\tests\kill_real.rs inside `wb-emulate --profile emu-low` (default). Each case runs in
  its own test process (the kill is by engine file name):

    1. five kills between chunks: the supervisor runs `wb-engine batch` (avx2, small model) to the end once
       without interruption, then again while the engine is killed at five random points (seeded). The
       committed transcript after the kills must equal the uninterrupted one: same segment times, same
       text, no overlap, no repeat.
    2. split: the engine is told to split every chunk of 4 s or more (debug_force_split) and is killed three
       times right after it reports the first half done, i.e. inside the second half. The committed
       transcript must equal the uninterrupted forced-split run (no half lost, none repeated).
    3. slow: large-v3 q5_0 on a 60 s Hindi clip with 30 s chunks under the default 60 s watchdog. One chunk's
       decode must exceed 120 s and the supervisor must not restart the engine (the engine sends
       heartbeats from whisper's callbacks).
    4. detection: languages en+hi with language detection on (large-v3 q5_0) and a 20 s watchdog; the
       detection pass has no whisper callback, so the engine holds the heartbeats for it. The first chunk
       must take over 40 s to arrive and the engine must not be restarted.

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
  [string]$Emulate,
  [string]$SlowClipDir = 'D:\STELMAR-Gate\speed'
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
$wavShort = Join-Path $Scratch 'concat-150s.wav'
$outRun = Join-Path $Scratch "run-$Profile"
$outRunSplit = Join-Path $Scratch "run-split-$Profile"
$outRunSlow = Join-Path $Scratch "run-slow-$Profile"
$wavDetect = Join-Path $Scratch 'concat-hi-10s.wav'
$outRunDetect = Join-Path $Scratch "run-detect-$Profile"
New-Item -ItemType Directory -Force -Path $Scratch | Out-Null
foreach ($d in $outRun, $outRunSplit, $outRunSlow, $outRunDetect) { if (Test-Path $d) { Remove-Item $d -Recurse -Force } }
$engine = Join-Path $Scratch 'wb-engine-killtest.exe'
$engineSlow = Join-Path $Scratch 'wb-engine-slowtest.exe'
Copy-Item $engineSrc $engine -Force
Copy-Item $engineSrc $engineSlow -Force

$mk = Invoke-WbNative -FilePath $Python -ArgumentList @((Join-Path $here 'make_concat.py'), '--out', $wav)
if ($mk.ExitCode -ne 0) { throw "make_concat failed: $($mk.Output -join ' ') $($mk.Error -join ' ')" }
$wavLine = $mk.Output | Select-Object -Last 1
$mk2 = Invoke-WbNative -FilePath $Python -ArgumentList @((Join-Path $here 'make_concat.py'), '--out', $wavShort, '--seconds', '150')
if ($mk2.ExitCode -ne 0) { throw "make_concat (150 s) failed: $($mk2.Output -join ' ') $($mk2.Error -join ' ')" }
$wavShortLine = $mk2.Output | Select-Object -Last 1
$mk3 = Invoke-WbNative -FilePath $Python -ArgumentList @((Join-Path $here 'make_concat.py'), '--out', $wavDetect, '--seconds', '10', '--dataset', 'fleurs-hi')
if ($mk3.ExitCode -ne 0) { throw "make_concat (hi, 10 s) failed: $($mk3.Output -join ' ') $($mk3.Error -join ' ')" }
$wavDetectLine = $mk3.Output | Select-Object -Last 1

# Build the test binary (a build: it must not overlap a timing-sensitive run, and none is running).
$build = Invoke-WbNative -FilePath 'cargo' -ArgumentList @('test', '-p', 'wb-supervisor', '--test', 'kill_real', '--no-run', '--message-format=json')
if ($build.ExitCode -ne 0) { throw "cargo test --no-run failed: $($build.Error -join ' ')" }
$testExe = $null
foreach ($l in $build.Output) { if ($l -match '"executable":"([^"]+kill_real[^"]+\.exe)"') { $testExe = ($Matches[1] -replace '\\\\', '\') } }
if (-not $testExe -or -not (Test-Path $testExe)) { throw 'could not find the kill_real test executable' }

function Invoke-KillTest([string]$TestName, [string]$ReportDir, [hashtable]$EnvVars) {
  foreach ($k in $EnvVars.Keys) { Set-Item -Path "Env:$k" -Value $EnvVars[$k] }
  $argsList = if ($Profile -eq 'native') { @($TestName, '--exact', '--ignored', '--nocapture') } else { @('--profile', $Profile, '--log', (Join-Path $Scratch "emulate-$Profile.log"), '--', $testExe, $TestName, '--exact', '--ignored', '--nocapture') }
  $exe = if ($Profile -eq 'native') { $testExe } else { $Emulate }
  $started = Get-Date
  $run = Invoke-WbNative -FilePath $exe -ArgumentList $argsList
  $seconds = [int]((Get-Date) - $started).TotalSeconds
  $rp = Join-Path $ReportDir 'report.txt'
  $report = @()
  if (Test-Path $rp) { $report = Get-Content $rp }
  [pscustomobject]@{ Run = $run; Seconds = $seconds; Report = $report }
}
function Get-Sha([string]$File) { (Get-FileHash $File -Algorithm SHA256).Hash.ToLowerInvariant() }

$vad = Join-Path $repo 'models\ggml-silero-v6.2.0.bin'
$small = Join-Path $repo 'models\ggml-small.bin'
$large = Join-Path $repo 'models\ggml-large-v3-q5_0.bin'
$hiClip = Join-Path $SlowClipDir 'clip-hi.wav'

# 1. Five kills between chunks (5-minute English clip).
$r1 = Invoke-KillTest 'five_random_kills_leave_the_transcript_unchanged' $outRun @{
  WB_KILLTEST_ENGINE = $engine; WB_KILLTEST_WAV = $wav; WB_KILLTEST_MODEL = $small; WB_KILLTEST_VAD = $vad
  WB_KILLTEST_OUT = $outRun; WB_KILLTEST_THREADS = "$Threads" }

# 2. Kills inside the second half of a forced split (2.5-minute English clip).
$r2 = Invoke-KillTest 'kills_inside_the_second_half_of_a_split_chunk_lose_and_repeat_no_half' (Join-Path $outRunSplit 'split') @{
  WB_KILLTEST_ENGINE = $engine; WB_KILLTEST_WAV = $wavShort; WB_KILLTEST_MODEL = $small; WB_KILLTEST_VAD = $vad
  WB_KILLTEST_OUT = $outRunSplit; WB_KILLTEST_THREADS = "$Threads" }

# 3. A decode of over two minutes under the default watchdog (large-v3 q5_0, Hindi).
$r3 = $null
if ((Test-Path $large) -and (Test-Path $hiClip)) {
  $r3 = Invoke-KillTest 'a_decode_of_over_two_minutes_is_not_a_hang' $outRunSlow @{
    WB_SLOWTEST_ENGINE = $engineSlow; WB_SLOWTEST_WAV = $hiClip; WB_SLOWTEST_MODEL = $large; WB_SLOWTEST_VAD = $vad
    WB_SLOWTEST_OUT = $outRunSlow; WB_SLOWTEST_THREADS = "$Threads" }
}

# 4. A language-detection pass longer than the watchdog (large-v3 q5_0, en+hi, detection on, 20 s watchdog).
$r4 = $null
if ((Test-Path $large) -and (Test-Path $wavDetect)) {
  $r4 = Invoke-KillTest 'a_language_detection_pass_longer_than_the_watchdog_is_not_a_hang' $outRunDetect @{
    WB_SLOWTEST_ENGINE = $engineSlow; WB_DETECT_WAV = $wavDetect; WB_SLOWTEST_MODEL = $large; WB_SLOWTEST_VAD = $vad
    WB_DETECT_OUT = $outRunDetect; WB_SLOWTEST_THREADS = "$Threads" }
}

$lines = @(
  "Supervisor kill tests with the real engine (Phase 2), profile $Profile",
  "date: $((Get-Date).ToString('yyyy-MM-dd HH:mm', [Globalization.CultureInfo]::InvariantCulture)) (IST on the Legion)",
  ('overlay tools closed: ' + $(if ((Get-WbOverlayTools).Count -eq 0) { 'yes' } else { 'NO' })),
  'no engine build was running when the tests started (checked); the test binary was built before they ran',
  "engine: wb-engine avx2 (copy named wb-engine-killtest.exe, sha256 $(Get-Sha $engine)), $Threads threads, previous-text context off",
  'test file: crates/wb-supervisor/tests/kill_real.rs; kill points drawn with a fixed seed (20261110); the engine is killed with taskkill /F',
  '',
  "== Case 1: five kills between chunks (small f16, $($r1.Seconds) s, test exit code $($r1.Run.ExitCode)) ==",
  "audio: $wavLine (FLEURS English test utterances in id order joined with 0.8 s of silence; not committed)"
) + $r1.Report + @(
  '',
  "== Case 2: three kills inside the second half of a split chunk (small f16, $($r2.Seconds) s, test exit code $($r2.Run.ExitCode)) ==",
  "audio: $wavShortLine (the same construction, 150 s)"
) + $r2.Report
if ($r3) {
  $lines += @('', "== Case 3: a decode of over two minutes under the default 60 s watchdog (large-v3 q5_0, $($r3.Seconds) s, test exit code $($r3.Run.ExitCode)) ==",
    "engine: wb-engine avx2 (copy named wb-engine-slowtest.exe, sha256 $(Get-Sha $engineSlow)); audio: 60 s FLEURS Hindi clip (not committed)") + $r3.Report
} else {
  $lines += @('', 'Case 3 not run: the large-v3 q5_0 model or the Hindi clip is missing.')
}
if ($r4) {
  $lines += @('', "== Case 4: a language-detection pass longer than a 20 s watchdog (large-v3 q5_0, en+hi, detection on, $($r4.Seconds) s, test exit code $($r4.Run.ExitCode)) ==",
    "engine: wb-engine avx2 (the slowtest copy); audio: $wavDetectLine (FLEURS Hindi, not committed)") + $r4.Report
} else {
  $lines += @('', 'Case 4 not run: the large-v3 q5_0 model or the Hindi clip is missing.')
}
foreach ($r in @($r1, $r2, $r3, $r4)) {
  if ($r -and $r.Run.ExitCode -ne 0 -and $r.Report.Count -eq 0) { $lines += ($r.Run.Output | Select-Object -Last 15); $lines += ($r.Run.Error | Select-Object -Last 15) }
}
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$path = Join-Path $OutDir "supervisor-kill-$Profile.txt"
Write-WbEvidence -Path $path -Lines $lines
Write-Host "evidence: $path"
$lines | Select-Object -Last 12
if (($r1.Run.ExitCode -ne 0) -or ($r2.Run.ExitCode -ne 0) -or (-not $r3) -or ($r3.Run.ExitCode -ne 0) -or (-not $r4) -or ($r4.Run.ExitCode -ne 0)) { exit 1 }
