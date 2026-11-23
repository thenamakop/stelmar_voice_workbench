# Gate supervisor-kill: the supervisor's kill tests with the real engine (Phase 2, crates\wb-supervisor\tests\kill_real.rs,
# pre-built into the pack as bin\kill_real.exe from the same source commit as the rest of the pack, so it carries
# Phase 4's queue and resume fixes). Four cases, each in its own test process (the kill is by engine file name):
#   1. five kills between chunks (small model, 5-minute English clip): the committed transcript equals an
#      uninterrupted run: same segment times, same text, no overlap, no repeat.
#   2. three kills inside the second half of a forced-split chunk (150 s clip): no half lost, none repeated.
#   3. a decode of over two minutes under the default 60 s watchdog (large-v3 q5_0, 60 s Hindi clip): no restart.
#   4. a language-detection pass longer than a 20 s watchdog (large-v3 q5_0, en+hi, 10 s Hindi clip): no restart.
# -Quick runs case 1 only. The test judges recovery only. Phase 4's day-sim -Kill (two 30-minute sessions through the
# cable, engines killed at seeded times, Python report) is a Legion-only check and is not part of this gate.
# The clips are public FLEURS test audio (CC BY 4.0), the same files the Phase 2 evidence used. The test never starts
# the app, but the gate still gives the run its own app data folder (WB_GATE_DATA_DIR) like every other gate, so
# nothing it starts can reach the real one. Windows PowerShell 5.1, ASCII only.
param([string]$CtxFile, [string]$ResultFile)
$ErrorActionPreference = 'Stop'
$lib = Join-Path $PSScriptRoot '..\..\lib'
Import-Module (Join-Path $lib 'PackCore.psm1') -Force
Import-Module (Join-Path $lib 'PackGate.psm1') -Force
$ctx = Read-PackCtx -CtxFile $CtxFile
Initialize-PackGate -Ctx $ctx
Import-Module (Join-Path $lib 'GateData.psm1') -Force
Import-Module (Join-Path $lib 'PackNative.psm1') -Force
$paths = Get-PackPaths -Ctx $ctx

Invoke-PackGateBody -Ctx $ctx -Id 'supervisor-kill' -ResultFile $ResultFile -Body {
  $quick = [bool]$ctx.quick
  $profile = [string]$ctx.profile
  $isEmu = $profile -ne 'laptop'
  $thr = 'the supervisor restarts the engine; the committed segments equal an uninterrupted run; no segment lost or repeated; no watchdog kill in the long-decode cases'
  $triple = 'x86_64-pc-windows-msvc'
  $work = Join-Path ([string]$ctx.gate_dir) 'work'
  New-Item -ItemType Directory -Force -Path $work | Out-Null
  # The run's own app data folder, inside the gate's folder (never the real one).
  $null = Set-WbGateDataDir -Dir (Join-Path ([string]$ctx.gate_dir) 'appdata')
  Assert-WbGateDataDir -Gate 'supervisor-kill'

  $testExe = Join-Path $paths.Root 'bin\kill_real.exe'
  $engineSrc = Join-Path $paths.Engines "wb-engine-avx2-$triple.exe"
  $small = Join-Path $paths.Models 'ggml-small.bin'
  $large = Join-Path $paths.Models 'ggml-large-v3-q5_0.bin'
  $vad = Join-Path $paths.Models 'ggml-silero-v6.2.0.bin'
  $wav5 = Join-Path $paths.Data 'kill\concat-5min.wav'
  $wav150 = Join-Path $paths.Data 'kill\concat-150s.wav'
  $wavHi60 = Join-Path $paths.Data 'kill\clip-hi.wav'
  $wavHi10 = Join-Path $paths.Data 'kill\concat-hi-10s.wav'
  $need = @($testExe, $engineSrc, $small, $vad, $wav5)
  if (-not $quick) { $need += @($wav150, $large, $wavHi60, $wavHi10) }
  if ($isEmu -and -not (Test-Path -LiteralPath $paths.Emulate)) { throw "the emulator is missing: $($paths.Emulate)" }
  foreach ($f in $need) { if (-not (Test-Path -LiteralPath $f)) { throw "supervisor-kill cannot start: $f is missing from the pack" } }

  $threads = 3
  if (-not $isEmu) { $threads = [math]::Max(1, [math]::Min(3, [int]([Environment]::ProcessorCount) - 1)) }
  # The engine is killed by file name: use copies with names no other engine has.
  $engine = Join-Path $work 'wb-engine-killtest.exe'
  $engineSlow = Join-Path $work 'wb-engine-slowtest.exe'
  Copy-Item -LiteralPath $engineSrc -Destination $engine -Force
  Copy-Item -LiteralPath $engineSrc -Destination $engineSlow -Force

  function Invoke-KillCase([string]$Name, [string]$Test, [string]$ReportDir, [hashtable]$EnvVars, [int]$TimeoutMin) {
    if (Test-Path -LiteralPath $ReportDir) { Remove-Item -LiteralPath $ReportDir -Recurse -Force }
    foreach ($k in $EnvVars.Keys) { Set-Item -Path "Env:$k" -Value $EnvVars[$k] }
    $testArgs = @($Test, '--exact', '--ignored', '--nocapture')
    if ($isEmu) { $exe = $paths.Emulate; $a = @('--profile', $profile, '--log', (Join-Path $work "emulate-$Name.log"), '--', $testExe) + $testArgs }
    else { $exe = $testExe; $a = $testArgs }
    $t0 = Get-Date
    $r = Invoke-PackNative -FilePath $exe -ArgumentList $a -TimeoutSeconds ($TimeoutMin * 60)
    foreach ($k in $EnvVars.Keys) { Remove-Item -Path "Env:$k" -ErrorAction SilentlyContinue }
    $report = @()
    $rp = Join-Path $ReportDir 'report.txt'
    if (Test-Path -LiteralPath $rp) { $report = @(Get-Content -LiteralPath $rp -Encoding UTF8) }
    [pscustomobject]@{ Name = $Name; Exit = $r.ExitCode; Seconds = [int]((Get-Date) - $t0).TotalSeconds; Report = $report; Out = $r.Output; Err = $r.Error }
  }

  $out1 = Join-Path $work 'run-1'; $out2 = Join-Path $work 'run-2'; $out3 = Join-Path $work 'run-3'; $out4 = Join-Path $work 'run-4'
  $cases = New-Object System.Collections.Generic.List[object]
  $cases.Add((Invoke-KillCase 'case-1-five-kills' 'five_random_kills_leave_the_transcript_unchanged' $out1 @{
    WB_KILLTEST_ENGINE = $engine; WB_KILLTEST_WAV = $wav5; WB_KILLTEST_MODEL = $small; WB_KILLTEST_VAD = $vad
    WB_KILLTEST_OUT = $out1; WB_KILLTEST_THREADS = "$threads" } 120))
  if (-not $quick) {
    $cases.Add((Invoke-KillCase 'case-2-forced-split' 'kills_inside_the_second_half_of_a_split_chunk_lose_and_repeat_no_half' (Join-Path $out2 'split') @{
      WB_KILLTEST_ENGINE = $engine; WB_KILLTEST_WAV = $wav150; WB_KILLTEST_MODEL = $small; WB_KILLTEST_VAD = $vad
      WB_KILLTEST_OUT = $out2; WB_KILLTEST_THREADS = "$threads" } 90))
    $cases.Add((Invoke-KillCase 'case-3-long-decode' 'a_decode_of_over_two_minutes_is_not_a_hang' $out3 @{
      WB_SLOWTEST_ENGINE = $engineSlow; WB_SLOWTEST_WAV = $wavHi60; WB_SLOWTEST_MODEL = $large; WB_SLOWTEST_VAD = $vad
      WB_SLOWTEST_OUT = $out3; WB_SLOWTEST_THREADS = "$threads" } 60))
    $cases.Add((Invoke-KillCase 'case-4-detection' 'a_language_detection_pass_longer_than_the_watchdog_is_not_a_hang' $out4 @{
      WB_SLOWTEST_ENGINE = $engineSlow; WB_DETECT_WAV = $wavHi10; WB_SLOWTEST_MODEL = $large; WB_SLOWTEST_VAD = $vad
      WB_DETECT_OUT = $out4; WB_SLOWTEST_THREADS = "$threads" } 30))
  }

  $lines = New-Object System.Collections.Generic.List[string]
  $lines.Add('Supervisor kill tests with the real engine (Phase 2 test file, built from the pack''s source commit)')
  $lines.Add(('machine: {0}; profile: {1}; date: {2}; threads: {3}; engine: wb-engine avx2 (copy named wb-engine-killtest.exe); {4}' -f $ctx.machine, $profile, (Get-Date -Format 'yyyy-MM-dd HH:mm'), $threads, $(if ($quick) { 'QUICK: case 1 only, not at full length' } else { 'all four cases' })))
  $lines.Add('test: bin\kill_real.exe (crates/wb-supervisor/tests/kill_real.rs); kill points drawn with the fixed seed 20261110; the engine is killed with taskkill /F; the verdict is recovery only')
  $lines.Add('not in this gate: Phase 4''s day-sim -Kill (two 30-minute sessions, engines killed at seeded times, Python report) is a Legion-only check')
  $failed = New-Object System.Collections.Generic.List[string]
  foreach ($c in $cases) {
    $lines.Add('')
    $lines.Add(('== {0}: exit {1}, {2} s ==' -f $c.Name, $c.Exit, $c.Seconds))
    foreach ($l in $c.Report) { $lines.Add($l) }
    if ($c.Exit -ne 0) {
      $failed.Add($c.Name)
      if ($c.Report.Count -eq 0) { foreach ($l in @($c.Out | Select-Object -Last 15)) { $lines.Add("   out: $l") }; foreach ($l in @($c.Err | Select-Object -Last 15)) { $lines.Add("   err: $l") } }
    }
  }
  $status = $(if ($failed.Count -gt 0) { 'FAIL' } else { 'PASS' })
  $lines.Add('')
  $lines.Add('verdict: ' + $status)
  $ev = Join-Path ([string]$ctx.gate_dir) 'supervisor-kill.txt'
  Set-Content -LiteralPath $ev -Value @($lines) -Encoding UTF8
  $head = $(if ($failed.Count -gt 0) { ('{0} of {1} case(s) failed: {2}' -f $failed.Count, $cases.Count, ($failed -join ', ')) } else { ('{0} case(s) passed' -f $cases.Count) })
  New-PackResult -Id 'supervisor-kill' -Status $status -Headline $head -Measured ([ordered]@{ cases = $cases.Count; failed = $failed.Count; seconds = (($cases | Measure-Object -Property Seconds -Sum).Sum) }) -Threshold $thr -Evidence @($ev) -Quick $quick -NotAtFullLength $quick
}
