<#
.SYNOPSIS
  Why the queue's units ran at 1.45 to 1.8 times real time when the profile promised 0.69, and why the night unit's
  memory peak was higher than the profile's (owner, 17 Nov 2026, steps 2 and 3).

.DESCRIPTION
  Six engine runs under `wb-emulate --profile emu-low` on a quiet machine, each timed and sampled once a second
  (working set, commit, the engine's CPU, the host's total CPU):
    P1  the profile cell itself: `wb-engine bench`, Hindi 60 s clip, turbo q5_0, the Vulkan UHD, 2 threads, rolling
    Q   the queue's own rolling request of the second smoke (unit 1): batch, English + Hindi, language detection on
    R   the same unit as the cell asks for it: batch, Hindi only, no language detection
    P2  the profile cell again (drift of the machine over the session)
    N2  the night model alone, Hindi only, no detection, first 120 s
    N1  the queue's night request of the second smoke (unit 0): turbo for English and large-v3 q5_0 for Hindi, detection on
  Quiet-machine rule, before every run: no stelmar-*.lock held by anyone else (this script takes the gate lock),
  then 60 s of sampling must show the host's total CPU under 10% and no other pwsh, cargo, robocopy, python or wb-*
  process busy. If it is not idle the script waits a minute and checks again (up to 30 minutes), and never starts a
  run on a busy machine. Each idle check goes in the evidence.

  Needs the second smoke's run folder (its unit requests and audio). Holds the gate lock for the whole script, about
  45 minutes. Evidence: docs\evidence\phase-4\speed-diagnosis-emu-low.txt (through scripts\lib\Redact.psm1).
#>
[CmdletBinding()]
param(
  [string]$SmokeRun = 'D:\STELMAR-Gate\day-sim\smoke2-emu-low-20261117-150935',
  [string]$Work = 'D:\STELMAR-Gate\speed-diag',
  [string]$OutFile,
  [string]$Emulate,
  [string]$Machine = 'Legion',
  [string]$EmuProfile = 'emu-low',
  [double]$IdleCpuMax = 10,
  [int]$IdleSeconds = 60,
  [int]$IdleWaitMin = 30,
  [string[]]$Runs = @('P1', 'Q', 'R', 'P2', 'N2', 'N1'),
  # Earlier figures to compare with, e.g. 'P1=0.708,P2=0.709,Q=1.011' (the recheck of 18 Nov 2026, after Wallpaper Engine was quit).
  [string]$Compare = ''
)
$ErrorActionPreference = 'Stop'
# `-File` hands -Runs P1,P2,Q over as one string: split it.
$Runs = @($Runs | ForEach-Object { $_ -split ',' } | Where-Object { $_ })
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo = (Resolve-Path (Join-Path $here '..\..')).Path
if (-not $OutFile) { $OutFile = Join-Path $repo 'docs\evidence\phase-4\speed-diagnosis-emu-low.txt' }
if (-not $Emulate) { $Emulate = Join-Path $repo 'target\release\wb-emulate.exe' }
Import-Module (Join-Path $repo 'scripts\lib\PsEnv.psm1') -Force
Set-WbCleanPSModulePath
Set-WbInvariantCulture
Import-Module (Join-Path $repo 'scripts\lib\GateRun.psm1') -Force
Import-Module (Join-Path $repo 'scripts\lib\Redact.psm1') -Force
$python = Join-Path $repo 'eval\.venv\Scripts\python.exe'
$triple = 'x86_64-pc-windows-msvc'
$minPath = "$env:SystemRoot\System32;$env:SystemRoot"
$bench = Join-Path $repo 'resources\bench\bench-hi.wav'
$src = Join-Path $repo "src-tauri\binaries\wb-engine-vulkan-$triple.exe"
$session = Get-ChildItem -Path (Join-Path $SmokeRun 'rec') -Directory | ForEach-Object { Get-ChildItem $_.FullName -Directory } | Select-Object -First 1
foreach ($f in @($bench, $src, $Emulate, $python, $SmokeRun, $session.FullName)) { if (-not (Test-Path -LiteralPath $f)) { throw "missing: $f" } }
New-Item -ItemType Directory -Force -Path $Work, (Split-Path -Parent $OutFile) | Out-Null
Get-ChildItem -Path $Work -File -ErrorAction SilentlyContinue | Remove-Item -Force

$prep = & $python -I (Join-Path $here 'speed_diag.py') prep $session.FullName $Work $bench
Write-Host "prepared: $prep"
$engDir = Join-Path $Work 'engine'
New-Item -ItemType Directory -Force -Path $engDir | Out-Null
$exe = Join-Path $engDir 'wb-engine-speed-diag.exe'
Copy-Item -LiteralPath $src -Destination $exe -Force
$engSha = (Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash.ToLowerInvariant()
$procName = 'wb-engine-speed-diag'
$cpuCounter = New-Object Diagnostics.PerformanceCounter('Processor', '% Processor Time', '_Total')
$null = $cpuCounter.NextValue()
$busyNames = '^(pwsh|powershell|cargo|rustc|robocopy|python|git|link|cl|msbuild|cmake|ninja|node|wb-.*|stelmar.*)$'

# 60 s of quiet: the host's total CPU and every other process's CPU time over the window.
function Measure-Idle([string]$Label) {
  $started = Get-Date
  $attempts = 0
  $deadline = $started.AddMinutes($IdleWaitMin)
  while ($true) {
    $attempts++
    # Any other stelmar lock or the gate-pack's request flag: wait, never measure beside it.
    $others = @('D:\Projects\stelmar-build.lock', 'D:\Projects\stelmar-gate-request.flag') | Where-Object { Test-Path -LiteralPath $_ }
    if ($others) {
      if ((Get-Date) -gt $deadline) { throw "$($others -join ', ') still present after $IdleWaitMin minutes; no measurement was made before run $Label" }
      Write-Host "$(Get-Date -Format 'HH:mm:ss') $Label : $($others -join ', ') present; waiting a minute"
      Start-Sleep -Seconds 60
      continue
    }
    $before = @{}
    foreach ($pr in Get-Process) { try { $before[$pr.Id] = [pscustomobject]@{ Name = $pr.ProcessName; Cpu = $pr.TotalProcessorTime.TotalSeconds } } catch { } }
    $cpu = New-Object Collections.Generic.List[double]
    $null = $cpuCounter.NextValue()
    $sw = [Diagnostics.Stopwatch]::StartNew()
    for ($i = 1; $i -le $IdleSeconds; $i++) {
      $wait = $i * 1000 - $sw.ElapsedMilliseconds
      if ($wait -gt 0) { Start-Sleep -Milliseconds $wait }
      $cpu.Add([double]$cpuCounter.NextValue())
    }
    $top = New-Object Collections.Generic.List[object]
    foreach ($pr in Get-Process) {
      if ($pr.Id -eq $PID) { continue }
      try {
        $b = $before[$pr.Id]
        if ($b -and $b.Name -eq $pr.ProcessName) {
          $d = $pr.TotalProcessorTime.TotalSeconds - $b.Cpu
          if ($d -ge 0.5) { $top.Add([pscustomobject]@{ name = $pr.ProcessName; cpu_s = [math]::Round($d, 2) }) }
        }
      } catch { }
    }
    $sorted = @($top | Sort-Object cpu_s -Descending)
    $busy = @($sorted | Where-Object { $_.name -match $busyNames -and $_.cpu_s -ge 3 } | ForEach-Object { "$($_.name) $($_.cpu_s) CPU-s" })
    $avg = ($cpu | Measure-Object -Average).Average
    $max = ($cpu | Measure-Object -Maximum).Maximum
    $ok = ($avg -lt $IdleCpuMax) -and ($busy.Count -eq 0)
    $rec = [ordered]@{
      at = (Get-Date -Format 'yyyy-MM-dd HH:mm:ss'); seconds = $IdleSeconds; attempts = $attempts
      cpu_avg = [math]::Round($avg, 1); cpu_max = [math]::Round($max, 1); busy = $busy; top = $sorted
      free_ram_gb = (Get-WbFreeRamGB); ok = $ok
    }
    ($rec | ConvertTo-Json -Depth 5) | Set-Content -LiteralPath (Join-Path $Work "idle-$Label.json") -Encoding utf8
    Write-Host ("{0} idle check before {1}: host CPU {2:N1}% (max {3:N0}%), busy [{4}] -> {5}" -f (Get-Date -Format 'HH:mm:ss'), $Label, $avg, $max, ($busy -join ', '), $(if ($ok) { 'IDLE' } else { 'not idle' }))
    if ($ok) { return }
    if ((Get-Date) -gt $deadline) { throw "the machine did not become idle within $IdleWaitMin minutes before run $Label; no measurement was made on a busy machine" }
    Write-Host "$(Get-Date -Format 'HH:mm:ss') waiting a minute, then checking again"
    Start-Sleep -Seconds 60
  }
}

function Invoke-Traced([string]$Label, [string[]]$EngineArgs, [int]$TimeoutMin) {
  $o = Join-Path $Work "$Label.out"; $er = Join-Path $Work "$Label.err"; $csv = Join-Path $Work "$Label.trace.csv"
  $argList = @('--profile', $EmuProfile, '--', ('"' + $exe + '"')) + $EngineArgs
  $rows = New-Object Collections.Generic.List[string]
  $rows.Add('t_s,ws_mib,commit_mib,engine_cpu_pct,host_cpu_pct')
  $saved = $env:PATH
  $env:PATH = $minPath
  $startedAt = Get-Date -Format 'yyyy-MM-dd HH:mm:ss'
  try { $p = Start-Process -FilePath $Emulate -ArgumentList $argList -PassThru -WindowStyle Hidden -RedirectStandardOutput $o -RedirectStandardError $er }
  finally { $env:PATH = $saved }
  $null = $p.Handle
  $null = $cpuCounter.NextValue()
  $sw = [Diagnostics.Stopwatch]::StartNew()
  $prevCpu = $null; $prevT = 0.0
  $timedOut = $false
  $tick = 0
  while (-not $p.HasExited) {
    if ($sw.Elapsed.TotalMinutes -ge $TimeoutMin) {
      $timedOut = $true
      Get-Process -Name $procName -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
      try { $p.Kill() } catch { }
      break
    }
    $tick++
    $wait = $tick * 1000 - $sw.ElapsedMilliseconds
    if ($wait -gt 0) { Start-Sleep -Milliseconds $wait }
    $t = $sw.Elapsed.TotalSeconds
    $hostCpu = [math]::Round([double]$cpuCounter.NextValue(), 1)
    $e = Get-Process -Name $procName -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($e) {
      try {
        if ($e.PriorityClass -ne [Diagnostics.ProcessPriorityClass]::BelowNormal) { $e.PriorityClass = [Diagnostics.ProcessPriorityClass]::BelowNormal }
        $cpuNow = $e.TotalProcessorTime.TotalSeconds
        $pct = 0.0
        if ($null -ne $prevCpu -and $t -gt $prevT) { $pct = ($cpuNow - $prevCpu) / ($t - $prevT) * 100 }
        $prevCpu = $cpuNow; $prevT = $t
        # F formats have no thousands separator (N formats made "1,135" and broke the CSV).
        $rows.Add(('{0:F1},{1:F0},{2:F0},{3:F0},{4:F1}' -f $t, ($e.WorkingSet64 / 1MB), ($e.PrivateMemorySize64 / 1MB), $pct, $hostCpu))
      } catch { }
    }
  }
  $p.WaitForExit()
  [IO.File]::WriteAllLines($csv, $rows)
  $meta = [ordered]@{ label = $Label; exit = $(if ($timedOut) { -2 } else { $p.ExitCode }); wall_s = [math]::Round($sw.Elapsed.TotalSeconds, 1); started = $startedAt; engine_sha256 = $engSha }
  ($meta | ConvertTo-Json) | Set-Content -LiteralPath (Join-Path $Work "$Label.run.json") -Encoding utf8
  Write-Host ("{0} run {1}: exit {2}, {3} s" -f (Get-Date -Format 'HH:mm:ss'), $Label, $meta.exit, $meta.wall_s)
  $meta
}

Invoke-WbGate -Name 'speed-diagnosis' -Body {
  $cmd = @{
    P1 = @('bench', '--request', ('"' + (Join-Path $Work 'req-P.json') + '"'))
    P2 = @('bench', '--request', ('"' + (Join-Path $Work 'req-P.json') + '"'))
    Q  = @('batch', '--request', ('"' + (Join-Path $Work 'req-Q.json') + '"'))
    R  = @('batch', '--request', ('"' + (Join-Path $Work 'req-R.json') + '"'))
    N1 = @('batch', '--request', ('"' + (Join-Path $Work 'req-N1.json') + '"'))
    N2 = @('batch', '--request', ('"' + (Join-Path $Work 'req-N2.json') + '"'))
  }
  $limit = @{ P1 = 15; P2 = 15; Q = 25; R = 20; N1 = 40; N2 = 20 }
  foreach ($k in $Runs) {
    Measure-Idle -Label $k
    $null = Invoke-Traced -Label $k -EngineArgs $cmd[$k] -TimeoutMin $limit[$k]
  }
} | Out-Null

$header = Join-Path $Work 'header.txt'
$lines = @()
$lines += 'Speed and memory diagnosis under emu-low (owner, 17 Nov 2026, steps 2 and 3): the profile cell against the queue'
$lines += "date: $(Get-Date -Format 'yyyy-MM-dd HH:mm') (IST on the Legion)"
$lines += "machine: $Machine, profile $EmuProfile (4 logical processors, 11 GiB job limit)"
$lines += "command: powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\gates\speed-diagnosis.ps1"
$lines += "source: $((Invoke-WbNative -FilePath 'git' -ArgumentList @('-C', $repo, 'rev-parse', '--short', 'HEAD')).Output -join '')"
$lines += "audio and requests: the second smoke's session folder (units of post-1 and post-2), copied requests in the work folder; $prep"
$lines += "engine: wb-engine vulkan build (the app's), sha256 $engSha; wb-emulate $EmuProfile; engine at below-normal priority as the queue runs it; one process per run"
$lines | Set-Content -LiteralPath $header -Encoding utf8
$report = Join-Path $Work 'report.txt'
$reportArgs = @('report', $Work, $report, '--header', $header)
if ($Compare) { $reportArgs += @('--compare', $Compare) }
& $python -X utf8 -I (Join-Path $here 'speed_diag.py') @reportArgs | Out-Null
Write-WbEvidence -Path $OutFile -Lines @(Get-Content -LiteralPath $report -Encoding UTF8) -MachineLabel $Machine
Write-Host "evidence: $OutFile"
