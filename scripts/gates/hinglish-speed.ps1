<#
.SYNOPSIS
  Phase 5 speed of rule (d2), the Hinglish model for every chunk: wb-engine bench under emu-low, emu-mid and native.

.DESCRIPTION
  Same method as the Phase 2 speed tables (scripts/gates/speed-tables.ps1): wb-engine bench, 60 s FLEURS clips (the same
  clip-en.wav and clip-hi.wav), 1 warm-up and the median of 3 timed runs (warm-up plus 1 timed run where the warm-up RTF is
  over 2 under emulation), peak working set and commit sampled. Models: hinglish f16, q8_0 and q5_0 (converted locally,
  models\converted, never committed). Rows:
    A  emu-low   rolling on the CPU (avx2, 4 threads) and on the Intel UHD with Vulkan (2 CPU threads), English and Hindi;
                 near-live (greedy) on the CPU at half the threads (2), Hindi
    B  emu-mid   the same with 12 threads and 6 threads
    C  native    reference only (not a tier): rolling and near-live (the live stand-in) on the RTX 4070 with CUDA
  Results go to D:\STELMAR-Gate\speed-hinglishesults-<table>.jsonl as each cell finishes (a finished cell is skipped, so a
  stopped run resumes). Evidence: docs\evidence\phase-5\hinglish-speed-<table>.txt, written by hinglish_speed_report.py from
  the saved results. Timing-sensitive: holds the gate lock, refuses with overlay tools open or a build running.
#>
[CmdletBinding()]
param(
  [ValidateSet('A', 'B', 'C', 'V')] [string[]]$Tables = @('A', 'B', 'C'),
  [string]$Work = 'D:\STELMAR-Gate\speed-hinglish',
  [string]$OutDir,
  [string]$Emulate,
  [string]$Python,
  [string]$Machine = 'Legion',
  [int]$CellTimeoutMin = 60,
  [int]$Runs = 3,
  [string]$Cell,
  [switch]$ReportOnly
)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo = (Resolve-Path (Join-Path $here '..\..')).Path
if (-not $OutDir) { $OutDir = Join-Path $repo 'docs\evidence\phase-5' }
if (-not $Emulate) { $Emulate = Join-Path $repo 'target\release\wb-emulate.exe' }
if (-not $Python) { $Python = Join-Path $repo 'eval\.venv\Scripts\python.exe' }
Import-Module (Join-Path $repo 'scripts\lib\PsEnv.psm1') -Force
Set-WbCleanPSModulePath
Set-WbInvariantCulture
Import-Module (Join-Path $repo 'scripts\lib\GateRun.psm1') -Force
Import-Module (Join-Path $repo 'scripts\lib\Redact.psm1') -Force
$inv = [Globalization.CultureInfo]::InvariantCulture
function F([double]$v, [int]$d = 2) { $v.ToString("F$d", $inv) }

$triple = 'x86_64-pc-windows-msvc'
$minPath = "$env:SystemRoot\System32;$env:SystemRoot"
$models = @(
  @{ Name = 'hinglish f16'; Id = 'hinglish-f16'; File = 'converted\ggml-hinglish-f16.bin' },
  @{ Name = 'hinglish q8_0'; Id = 'hinglish-q8_0'; File = 'converted\ggml-hinglish-q8_0.bin' },
  @{ Name = 'hinglish q5_0'; Id = 'hinglish-q5_0'; File = 'converted\ggml-hinglish-q5_0.bin' }
)
$vadPath = Join-Path $repo 'models\ggml-silero-v6.2.0.bin'
$tableDefs = @{
  A = @{ Profile = 'emu-low'; Cpu = @(4); NearLive = 2; Gpu = @(@{ Variant = 'vulkan'; Match = 'Intel|UHD'; Label = 'vulkan on the UHD'; NearLive = $false }); Headroom = $true }
  B = @{ Profile = 'emu-mid'; Cpu = @(12); NearLive = 6; Gpu = @(@{ Variant = 'vulkan'; Match = 'Intel|UHD'; Label = 'vulkan on the UHD'; NearLive = $false }); Headroom = $true }
  C = @{ Profile = 'native'; Cpu = @(); NearLive = 0; Gpu = @(@{ Variant = 'cuda'; Match = 'NVIDIA|RTX'; Label = 'cuda on the RTX'; NearLive = $true }); Headroom = $false }
  # 28 Nov 2026: the engine the release bundle ships for a discrete card (no CUDA engine is shipped). Only the shipped model.
  V = @{ Profile = 'native'; Cpu = @(); NearLive = 0; Gpu = @(@{ Variant = 'vulkan'; Match = 'NVIDIA|RTX'; Label = 'vulkan on the RTX'; NearLive = $true }); Headroom = $false; Only = @('hinglish-q8_0') }
}

New-Item -ItemType Directory -Force -Path $Work | Out-Null

function Get-Events($Lines) { @($Lines | Where-Object { $_ -match '^\{' } | ForEach-Object { try { $_ | ConvertFrom-Json } catch { $null } } | Where-Object { $_ }) }

function Write-TableEvidence([string]$Table) {
  $def = $tableDefs[$Table]
  $file = Join-Path $Work "results-$Table.jsonl"
  if (-not (Test-Path $file)) { return }
  New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
  $path = Join-Path $OutDir "hinglish-speed-$Table.txt"
  $r = Invoke-WbNative -FilePath $Python -ArgumentList @('-X', 'utf8', '-I', (Join-Path $here 'hinglish_speed_report.py'), '--results', $file, '--table', $Table, '--machine', $Machine, '--out', $path)
  if ($r.ExitCode -ne 0) { throw "hinglish_speed_report failed: $($r.Output -join ' ') $($r.Error -join ' ')" }
  Write-Host "evidence: $path"
}

if ($ReportOnly) { foreach ($t in $Tables) { Write-TableEvidence $t }; return }

# ---------------------------------------------------------------- inputs
foreach ($m in $models) { if (-not (Test-Path (Join-Path $repo "models\$($m.File)"))) { throw "model missing: models\$($m.File)" } }
if (-not (Test-Path $vadPath)) { throw "VAD model missing: $vadPath" }
if (-not (Test-Path $Emulate)) { throw "wb-emulate not built: $Emulate" }
$clips = @{}
$clipSeconds = @{}
foreach ($l in @('en', 'hi')) {
  $wav = Join-Path $Work "clip-$l.wav"
  if (-not (Test-Path $wav)) {
    $mk = Invoke-WbNative -FilePath $Python -ArgumentList @((Join-Path $here 'make_concat.py'), '--out', $wav, '--seconds', '60', '--dataset', "fleurs-$l")
    if ($mk.ExitCode -ne 0) { throw "make_concat failed for $l : $($mk.Output -join ' ') $($mk.Error -join ' ')" }
  }
  $clips[$l] = $wav
  $clipSeconds[$l] = [math]::Round(((Get-Item $wav).Length - 44) / 2 / 16000, 1)
}

# Engine copies, one folder and one unique exe name per variant (the exe is found by name for the memory samples).
$engines = @{}
foreach ($v in @('avx2', 'vulkan', 'cuda')) {
  $src = Join-Path $repo "src-tauri\binaries\wb-engine-$v-$triple.exe"
  if (-not (Test-Path $src)) { Write-Host "engine $v not built: its rows are skipped"; continue }
  $dir = Join-Path $Work "engine-$v"
  New-Item -ItemType Directory -Force -Path $dir | Out-Null
  $exe = Join-Path $dir "wb-engine-speed-$v.exe"
  Copy-Item $src $exe -Force
  if ($v -eq 'cuda' -and $env:CUDA_PATH) {
    foreach ($b in @((Join-Path $env:CUDA_PATH 'bin'), (Join-Path $env:CUDA_PATH 'bin\x64'))) {
      if (Test-Path $b) { Get-ChildItem $b -Filter '*.dll' -ErrorAction SilentlyContinue | Where-Object { $_.Name -match '^(cudart|cublas)' } | ForEach-Object { Copy-Item $_.FullName $dir -Force } }
    }
  }
  $engines[$v] = @{ Exe = $exe; Name = "wb-engine-speed-$v"; Sha = (Get-FileHash $exe -Algorithm SHA256).Hash.ToLowerInvariant() }
}

function Invoke-Engine([string]$Profile, [string]$Variant, [string[]]$EngineArgs, [int]$TimeoutMin) {
  $e = $engines[$Variant]
  $o = Join-Path $Work "cell.out"; $er = Join-Path $Work "cell.err"
  $argList = $(if ($Profile -eq 'native') { $EngineArgs } else { @('--profile', $Profile, '--', ('"' + $e.Exe + '"')) + $EngineArgs })
  $file = $(if ($Profile -eq 'native') { $e.Exe } else { $Emulate })
  $saved = $env:PATH
  $env:PATH = $minPath
  try { $p = Start-Process -FilePath $file -ArgumentList $argList -PassThru -WindowStyle Hidden -RedirectStandardOutput $o -RedirectStandardError $er }
  finally { $env:PATH = $saved }
  $null = $p.Handle   # keeps the process handle open so ExitCode is readable after it exits
  $peakWs = 0L; $peakCommit = 0L
  $watch = [Diagnostics.Stopwatch]::StartNew()
  $timedOut = $false
  while (-not $p.HasExited) {
    foreach ($q in @(Get-Process -Name $e.Name -ErrorAction SilentlyContinue)) {
      try { if ($q.PeakWorkingSet64 -gt $peakWs) { $peakWs = $q.PeakWorkingSet64 }; if ($q.PeakPagedMemorySize64 -gt $peakCommit) { $peakCommit = $q.PeakPagedMemorySize64 } } catch { }
    }
    if ($watch.Elapsed.TotalMinutes -ge $TimeoutMin) {
      $timedOut = $true
      Get-Process -Name $e.Name -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
      try { $p.Kill() } catch { }
      break
    }
    Start-Sleep -Milliseconds 250
  }
  $p.WaitForExit()
  [pscustomobject]@{
    Exit = $(if ($timedOut) { -2 } else { $p.ExitCode }); TimedOut = $timedOut; Seconds = [math]::Round($watch.Elapsed.TotalSeconds, 1)
    Out = @(Get-Content $o -Encoding UTF8 -ErrorAction SilentlyContinue); Err = @(Get-Content $er -Encoding UTF8 -ErrorAction SilentlyContinue)
    PeakWs = $peakWs; PeakCommit = $peakCommit
  }
}

# GPU index of the first device of $Variant whose description matches, from that profile's own `info`.
$gpuIndexCache = @{}
function Get-GpuIndex([string]$Profile, [string]$Variant, [string]$Match) {
  $key = "$Profile|$Variant|$Match"
  if ($gpuIndexCache.ContainsKey($key)) { return $gpuIndexCache[$key] }
  $r = Invoke-Engine -Profile $Profile -Variant $Variant -EngineArgs @('info') -TimeoutMin 5
  $info = (Get-Events $r.Out | Where-Object { $_.event -eq 'info' } | Select-Object -First 1)
  $idx = $null
  if ($info) {
    $want = $(if ($Variant -eq 'cuda') { 'CUDA' } else { 'Vulkan' })
    $d = @($info.devices | Where-Object { $_.backend -eq $want -and $_.kind -ne 'cpu' -and (("$($_.description) $($_.name) $($_.adapter_name)") -match $Match) }) | Select-Object -First 1
    if ($d) { $idx = [int]$d.index }
  }
  $gpuIndexCache[$key] = $idx
  $idx
}

function Invoke-Cell($Table, $def, $row, $order, $variant, $device, $threads, $mode, $m, $lang, $modelOrder) {
  $profile = $def.Profile
  $key = "$Table|$row|$($m.Id)|$lang|$mode|$threads"
  if ($Cell -and $key -ne $Cell) { return }
  $resFile = Join-Path $Work "results-$Table.jsonl"
  if (Test-Path $resFile) {
    $done = @(Get-Content $resFile -Encoding UTF8 | Where-Object { $_ -like "*`"key`":`"$key`"*" } | ForEach-Object { $_ | ConvertFrom-Json } | Where-Object { $_.status -in @('ok', 'does_not_fit', 'timeout') })
    if ($done.Count) { Write-Host "skip (done): $key"; return }
  }
  $req = [ordered]@{
    type = 'bench'
    model = @{ id = $m.Id; path = (Join-Path $repo "models\$($m.File)") }
    vad_model = @{ id = 'silero-v6.2.0'; path = $vadPath }
    lang = $lang; clip = $clips[$lang]; mode = $mode; device = $device; threads = $threads; runs = $Runs
  }
  if ($profile -ne 'native') { $req.single_run_above_rtf = 2.0 }
  $reqFile = Join-Path $Work 'request.json'
  [IO.File]::WriteAllText($reqFile, ($req | ConvertTo-Json -Depth 6), (New-Object Text.UTF8Encoding($false)))
  Write-Host "$(Get-Date -Format 'HH:mm:ss') $key"
  $r = Invoke-Engine -Profile $profile -Variant $variant -EngineArgs @('bench', '--request', ('"' + $reqFile + '"')) -TimeoutMin $CellTimeoutMin
  $ev = Get-Events $r.Out
  $b = $ev | Where-Object { $_.event -eq 'bench' } | Select-Object -First 1
  $errEv = $ev | Where-Object { $_.event -eq 'error' } | Select-Object -First 1
  $rec = [ordered]@{
    key = $key; table = $Table; row = $row; order = $order; model = $m.Name; model_order = $modelOrder; lang = $lang; mode = $mode; threads = $threads
    variant = $variant; engine_sha = $engines[$variant].Sha; profile = $profile; when = (Get-Date).ToString('yyyy-MM-dd HH:mm', $inv)
    clip_en_s = $clipSeconds['en']; clip_hi_s = $clipSeconds['hi']; exit = $r.Exit; seconds = $r.Seconds
    peak_ws = $r.PeakWs; peak_commit = $r.PeakCommit
  }
  if ($b -and $r.Exit -eq 0) {
    $rec.status = 'ok'; $rec.rtf_median = $b.rtf_median; $rec.rtf_runs = @($b.rtf_runs); $rec.warmup_rtf = $b.warmup_rtf; $rec.single_run = [bool]$b.single_run
    $rec.load_s = $b.load_s; $rec.peak_ws = [int64]$b.peak_working_set_bytes; $rec.peak_commit = [int64]$b.peak_commit_bytes
    $rec.gpu_local_bytes = $b.gpu_local_bytes; $rec.gpu_shared_bytes = $b.gpu_shared_bytes; $rec.segments = $b.segments; $rec.truncated = $b.truncated_chunks
    $rec.whisper = "$($b.whisper_cpp.version) commit $($b.whisper_cpp.commit) tree $($b.whisper_cpp.tree_sha256)"; $rec.device_name = $b.device_name
  } else {
    $msg = $(if ($errEv) { "$($errEv.code): $($errEv.message)" } else { ($r.Err | Select-Object -Last 2) -join ' ' })
    $rec.status = $(if ($r.TimedOut) { 'timeout' } elseif ($errEv -and $errEv.code -match 'memory|alloc|oom') { 'does_not_fit' } elseif ($r.Exit -ne 0 -and -not $errEv -and $r.PeakCommit -gt 0.9 * 11GB) { 'does_not_fit' } elseif ($msg -match 'out of memory|alloc|bad_alloc|not enough memory|VK_ERROR_OUT_OF') { 'does_not_fit' } else { "exit_$($r.Exit)" })
    $rec.error = $msg -replace '[\r\n]+', ' '
  }
  ($rec | ConvertTo-Json -Compress -Depth 5) | Add-Content -Path $resFile -Encoding ascii
}

function Invoke-Table([string]$Table) {
  $def = $tableDefs[$Table]
  $profile = $def.Profile
  $order = 0
  # CPU rows: rolling (beam 5), English and Hindi
  foreach ($th in $def.Cpu) {
    $order++
    if (-not $engines.ContainsKey('avx2')) { continue }
    for ($i = 0; $i -lt $models.Count; $i++) { foreach ($l in @('en', 'hi')) { Invoke-Cell $Table $def "CPU avx2, $th threads" $order 'avx2' @{ kind = 'cpu' } $th 'rolling' $models[$i] $l $i } }
  }
  if ($Cell) { return }
  # GPU rows: rolling, English and Hindi (and near-live on a discrete card: the live stand-in)
  foreach ($g in $def.Gpu) {
    $order++
    if (-not $engines.ContainsKey($g.Variant)) { Write-Host "skip $($g.Label): engine not built"; continue }
    $idx = Get-GpuIndex $profile $g.Variant $g.Match
    if ($null -eq $idx) { Write-Host "skip $($g.Label): no matching device under $profile"; continue }
    $th = $(if ($profile -eq 'native') { 8 } else { 2 })
    for ($i = 0; $i -lt $models.Count; $i++) {
      if ($def.Only -and $models[$i].Id -notin $def.Only) { continue }
      foreach ($l in @('en', 'hi')) { Invoke-Cell $Table $def "$($g.Label) (device $idx), $th CPU threads" $order $g.Variant @{ kind = 'gpu'; index = $idx } $th 'rolling' $models[$i] $l $i }
      if ($g.NearLive) { Invoke-Cell $Table $def "$($g.Label) (device $idx), $th CPU threads, near-live (live stand-in)" ($order + 1) $g.Variant @{ kind = 'gpu'; index = $idx } $th 'near_live' $models[$i] 'hi' $i }
    }
  }
  # near-live on the CPU at half the threads, Hindi (the language that decides it)
  if ($def.NearLive -gt 0 -and $engines.ContainsKey('avx2')) {
    $order += 2
    for ($i = 0; $i -lt $models.Count; $i++) { Invoke-Cell $Table $def "CPU avx2, near-live (greedy), $($def.NearLive) threads" $order 'avx2' @{ kind = 'cpu' } $def.NearLive 'near_live' $models[$i] 'hi' $i }
  }
}

# ---------------------------------------------------------------- run
$requestFlag = 'D:\Projects\stelmar-gate-request.flag'
$ownsFlag = -not (Test-Path $requestFlag)
if ($ownsFlag) { Set-Content -LiteralPath $requestFlag -Value "requested by=speed tables`r`nstarted=$((Get-Date).ToString('o'))`r`npid=$PID" -Encoding ascii }
try {
  foreach ($t in $Tables) {
    Invoke-WbGate -Name "speed table $t" -Body { Invoke-Table $t }
    if (-not $Cell) { Write-TableEvidence $t }
  }
} finally {
  if ($ownsFlag) { Remove-Item -LiteralPath $requestFlag -Force -ErrorAction SilentlyContinue }
}
Write-Host 'speed tables done'
