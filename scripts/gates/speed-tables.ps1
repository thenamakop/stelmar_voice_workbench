<#
.SYNOPSIS
  Phase 2 speed tables A (emu-low), B (emu-mid) and C (native): wb-engine bench for every cell, resumable.

.DESCRIPTION
  Models: small, medium, large-v3-turbo q5_0, large-v3 q5_0. Clips: a 60 s FLEURS English clip and a 60 s FLEURS
  Hindi clip (utterances joined with 0.8 s of silence, built in D:\STELMAR-Gate\speed, never committed). Rolling
  settings (beam 5, temperature fallback, 15 s chunks); small also runs near-live (greedy) at half the logical threads.
  Each cell: 1 warm-up, then the median of 3 timed runs; under emu-low and emu-mid a cell whose warm-up RTF is over 2
  gets the warm-up plus one timed run and is marked "impractical: single run". A cell that cannot load or run inside
  the 11 GB job is recorded as "does not fit" with the error and the peak working set and commit, as a result.

  Rows (PLAN: speed tables):
    A  emu-low   avx2 at 3 and 4 threads;  vulkan on the UHD, 2 CPU threads;  small near-live at 2 threads
    B  emu-mid   avx2 at 9 and 12 threads; vulkan on the UHD, 2 CPU threads;  small near-live at 6 threads
    C  native    avx2 at 8 and 23 threads; cuda on the RTX; vulkan on the RTX; vulkan on the UHD (2 CPU threads);
                 small near-live at 16 threads. No headroom rule for C.

  Results go to D:\STELMAR-Gate\speed\results-<table>.jsonl as each cell finishes (a cell already in the file is
  skipped, so a stopped run resumes). Evidence: docs\evidence\phase-2\speed-table-<table>.txt, rewritten after every
  table. Timing-sensitive: holds the gate lock (Invoke-WbGate), refuses with overlay tools open or a build running,
  needs 14 GB free RAM. Use -ReportOnly to rewrite the evidence from the saved results.

  Smoke test: -Smoke runs one tiny set (small, English, 1 timed run, avx2) and writes nothing to docs.
#>
[CmdletBinding()]
param(
  [ValidateSet('A', 'B', 'C')] [string[]]$Tables = @('A', 'B', 'C'),
  [string]$Work = 'D:\STELMAR-Gate\speed',
  [string]$OutDir,
  [string]$Emulate,
  [string]$Python,
  [string]$Machine = 'Legion',
  [int]$CellTimeoutMin = 60,
  [int]$Runs = 3,
  [string]$Cell,   # run only the cell with this key (a re-check: use a separate -Work folder; no evidence file is written)
  [switch]$ReportOnly,
  [switch]$Smoke
)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo = (Resolve-Path (Join-Path $here '..\..')).Path
if (-not $OutDir) { $OutDir = Join-Path $repo 'docs\evidence\phase-2' }
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
  @{ Name = 'small'; Id = 'small'; File = 'ggml-small.bin' },
  @{ Name = 'medium'; Id = 'medium'; File = 'ggml-medium.bin' },
  @{ Name = 'large-v3-turbo q5_0'; Id = 'large-v3-turbo-q5_0'; File = 'ggml-large-v3-turbo-q5_0.bin' },
  @{ Name = 'large-v3 q5_0'; Id = 'large-v3-q5_0'; File = 'ggml-large-v3-q5_0.bin' }
)
$vadPath = Join-Path $repo 'models\ggml-silero-v6.2.0.bin'
$tableDefs = @{
  A = @{ Profile = 'emu-low'; Cpu = @(3, 4); NearLive = 2; Gpu = @(@{ Variant = 'vulkan'; Match = 'Intel|UHD'; Label = 'vulkan on the UHD' }); Headroom = $true }
  B = @{ Profile = 'emu-mid'; Cpu = @(9, 12); NearLive = 6; Gpu = @(@{ Variant = 'vulkan'; Match = 'Intel|UHD'; Label = 'vulkan on the UHD' }); Headroom = $true }
  C = @{ Profile = 'native'; Cpu = @(8, 23); NearLive = 16; Headroom = $false
    Gpu = @(@{ Variant = 'cuda'; Match = 'NVIDIA|RTX'; Label = 'cuda on the RTX' }, @{ Variant = 'vulkan'; Match = 'NVIDIA|RTX'; Label = 'vulkan on the RTX' }, @{ Variant = 'vulkan'; Match = 'Intel|UHD'; Label = 'vulkan on the UHD' }) }
}

New-Item -ItemType Directory -Force -Path $Work | Out-Null

function Get-Events($Lines) { @($Lines | Where-Object { $_ -match '^\{' } | ForEach-Object { try { $_ | ConvertFrom-Json } catch { $null } } | Where-Object { $_ }) }

# ---------------------------------------------------------------- reporting
function Get-CellText($r) {
  if ($r.status -eq 'does_not_fit') { return 'does not fit' }
  if ($r.status -ne 'ok') { return "FAILED ($($r.status))" }
  $t = F $r.rtf_median 3
  if ($r.single_run) { $t += ' (impractical: single run)' }
  $t
}

function Write-TableEvidence([string]$Table) {
  $def = $tableDefs[$Table]
  $file = Join-Path $Work "results-$Table.jsonl"
  if (-not (Test-Path $file)) { return }
  $all = @(Get-Content $file -Encoding UTF8 | Where-Object { $_.Trim() } | ForEach-Object { $_ | ConvertFrom-Json })
  $rows = @($all | Group-Object key | ForEach-Object { $_.Group | Select-Object -Last 1 })
  if (-not $rows.Count) { return }
  $lines = @()
  $first = $rows[0]
  $lines += "Speed table $Table - $($def.Profile) (Phase 2)"
  $lines += "date: $($first.when) to $(($rows | Select-Object -Last 1).when) (IST on the Legion); machine: $Machine; profile: $($def.Profile)$(if ($def.Profile -ne 'native') { ' (wb-emulate: affinity and 11 GB job limit, reported as 16 GB; UHD only)' } else { ' (no emulation)' })"
  $lines += "engines: " + ((($rows | ForEach-Object { "$($_.variant) sha256 $($_.engine_sha)" } | Sort-Object -Unique)) -join '; ')
  $lines += "whisper.cpp printed by the engine: " + ((($rows | Where-Object { $_.whisper } | ForEach-Object { $_.whisper } | Sort-Object -Unique)) -join '; ')
  $lines += "method: wb-engine bench, clip = 60 s of FLEURS test utterances joined with 0.8 s of silence (en: $(F $rows[0].clip_en_s 1) s, hi: $(F $rows[0].clip_hi_s 1) s), beam 5 + temperature fallback + 15 s VAD chunks ('rolling') except the near-live rows (greedy, utterance-length chunks); 1 warm-up, then the median of $Runs timed runs; RTF = processing time / clip length (load time excluded, reported apart); run from a folder with only the engine and a minimal PATH; Silero VAD v6.2.0 on the CPU"
  if ($def.Headroom) {
    $lines += 'headroom rule (LEGION-FIRST): the Legion emulates a laptop, so a model "meets" an RTF <= 1.0 bar only at a measured RTF <= 0.75, and an RTF <= 0.35 bar only at <= 0.26 (25% headroom). the last two columns apply that threshold to each row; no pass bar is set in Phase 2, the table feeds Phase 3.'
  } else {
    $lines += 'native table: no headroom rule applied (the Legion itself).'
  }
  $lines += 'under emu-low and emu-mid a cell whose warm-up RTF is over 2 gets the warm-up plus one timed run, marked "impractical: single run"; a cell that cannot load or run inside the 11 GB job is "does not fit" (a result, with the error and the peak memory).'
  $lines += ''
  $groups = $rows | Group-Object { $_.row }
  foreach ($g in ($groups | Sort-Object { $_.Group[0].order })) {
    $lines += "== $($g.Name)"
    $lines += 'model | lang | mode | threads | RTF median (runs) | warm-up RTF | load s | peak working set GiB | peak commit GiB | GPU local / shared GiB | segments | truncated chunks | meets RTF<=1.0 with headroom (<=0.75) | meets RTF<=0.35 with headroom (<=0.26)'
    foreach ($r in ($g.Group | Sort-Object model_order, lang, mode)) {
      if ($r.status -eq 'ok') {
        $gpu = $(if ($null -ne $r.gpu_local_bytes) { "$(F ($r.gpu_local_bytes / 1GB))/$(F ($r.gpu_shared_bytes / 1GB))" } else { 'n/a' })
        $m1 = $(if ($r.rtf_median -le 0.75) { 'yes' } elseif ($r.rtf_median -le 1.0) { 'no (within 25%)' } else { 'no' })
        $m2 = $(if ($r.rtf_median -le 0.26) { 'yes' } elseif ($r.rtf_median -le 0.35) { 'no (within 25%)' } else { 'no' })
        if (-not $def.Headroom) { $m1 = '-'; $m2 = '-' }
        $lines += "$($r.model) | $($r.lang) | $($r.mode) | $($r.threads) | $(Get-CellText $r) ($(($r.rtf_runs | ForEach-Object { F $_ 3 }) -join ', ')) | $(F $r.warmup_rtf 3) | $(F $r.load_s 1) | $(F ($r.peak_ws / 1GB)) | $(F ($r.peak_commit / 1GB)) | $gpu | $($r.segments) | $($r.truncated) | $m1 | $m2"
      } else {
        $lines += "$($r.model) | $($r.lang) | $($r.mode) | $($r.threads) | $(Get-CellText $r) | | | peak working set (sampled) $(F ($r.peak_ws / 1GB)) GiB | peak commit (sampled) $(F ($r.peak_commit / 1GB)) GiB | | | | exit code $($r.exit) $($r.error)"
      }
    }
    $lines += ''
  }
  $lines += "cells: $($rows.Count) (ok $(@($rows | Where-Object status -eq 'ok').Count), does not fit $(@($rows | Where-Object status -eq 'does_not_fit').Count), failed $(@($rows | Where-Object { $_.status -notin @('ok', 'does_not_fit') }).Count), single-run $(@($rows | Where-Object single_run).Count))"
  $lines += 'The Hinglish model is large-v3-sized: read its speed from the large-v3 q5_0 rows (the f16 model is slower and larger; the q5_0 Hinglish model is the same size as large-v3 q5_0).'
  $lines += ''
  $lines += 'Notes for Phase 3 (owner, 11 Nov 2026):'
  $lines += '(a) The Intel UHD of the Legion is weaker than the integrated GPUs the room laptops are likely to have, so every UHD row is a FLOOR: a laptop with Iris Xe, Arc or a Radeon 680M/780M is expected to be at least as fast. The exception is the low-baseline laptop (basic integrated graphics): it may be SLOWER than the Legion UHD, so for it these rows are an estimate to be re-measured there, not a bound.'
  $lines += '(b) Hindi costs more to decode than English on the same hardware: in these tables the Hindi clip runs 1.3 to 4.4 times slower than the English clip (about 3 times for large-v3 q5_0 on the UHD: 0.51 against about 1.5). The two clips are different recordings, so part of the gap belongs to the clip. Phase 3 tiers must consider turbo or medium for Hindi on weak machines.'
  $lines += '(c) On the UHD, DXGI reports the integrated GPU''s memory as local (the shared figure stays 0.00). That memory is system RAM: Phase 3 counts it as RAM (AGENTS.md: integrated GPUs share RAM with the CPU).'
  New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
  $path = $(if ($Smoke) { Join-Path $Work "smoke-$Table.txt" } else { Join-Path $OutDir "speed-table-$Table.txt" })
  Write-WbEvidence -Path $path -Lines $lines -MachineLabel $Machine
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
  $ms = @($(if ($Smoke) { $models[0] } else { $models }))
  $langs = @($(if ($Smoke) { 'en' } else { 'en', 'hi' }))
  $order = 0
  # CPU rows
  foreach ($th in @($(if ($Smoke) { $def.Cpu[0] } else { $def.Cpu }))) {
    $order++
    if (-not $engines.ContainsKey('avx2')) { continue }
    for ($i = 0; $i -lt $ms.Count; $i++) { foreach ($l in $langs) { Invoke-Cell $Table $def "CPU avx2, $th threads" $order 'avx2' @{ kind = 'cpu' } $th 'rolling' $ms[$i] $l $i } }
  }
  if ($Smoke -or $Cell) { return }
  # GPU rows
  foreach ($g in $def.Gpu) {
    $order++
    if (-not $engines.ContainsKey($g.Variant)) { Write-Host "skip $($g.Label): engine not built"; continue }
    $idx = Get-GpuIndex $profile $g.Variant $g.Match
    if ($null -eq $idx) { Write-Host "skip $($g.Label): no matching device under $profile"; continue }
    for ($i = 0; $i -lt $ms.Count; $i++) { foreach ($l in $langs) { Invoke-Cell $Table $def "$($g.Label) (device $idx), 2 CPU threads" $order $g.Variant @{ kind = 'gpu'; index = $idx } 2 'rolling' $ms[$i] $l $i } }
  }
  # small, near-live
  $order++
  foreach ($l in $langs) { Invoke-Cell $Table $def "CPU avx2, small near-live (greedy), $($def.NearLive) threads" $order 'avx2' @{ kind = 'cpu' } $def.NearLive 'near_live' $models[0] $l 0 }
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
