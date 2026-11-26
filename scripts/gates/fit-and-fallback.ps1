<#
.SYNOPSIS
  Phase 5, Hinglish rollout: (1) graphics-memory fit of the Hinglish model on the RTX 4070, (2) rule (c) against rule (a)
  and the Hinglish model's real batch speed under emu-low and emu-mid. Measurement only; nothing here is wired into the app.

.DESCRIPTION
  Part V (native, CUDA engine): `wb-engine batch` over a 120 s English + Hindi clip with one or two models loaded
  (Hinglish f16 or q8_0 alone, and together with large-v3-turbo-q5_0 as the second model, which the engine loads next to the
  first). nvidia-smi memory.used is sampled every 200 ms and the idle value before the run is subtracted: that is the
  video-memory peak of the engine process plus its CUDA context. The engine's own DXGI figures come from the bench cells
  (docs/evidence/phase-5/hinglish-speed-C.txt). The GPU is not throttled and no clock is changed.
  Part R (emu-low and emu-mid): `wb-emulate --profile P -- wb-engine batch` over the same clip, on the CPU (avx2) and on the
  Intel UHD (Vulkan), with rule (a) (today: language detection, one model per language) and rule (c) (every chunk decoded in
  both languages; a development engine built from commit f1d9251 with the `lang-experiments` feature, kept outside the
  repository and never bundled), with models en=medium and hi=large-v3-turbo-q5_0 (the rolling models the system check chose
  on the emulated laptops), and with the Hinglish f16 model for every chunk (rule (d2)) on a 60 s clip. RTF = decode seconds
  / audio seconds from the engine's `done` event; the bars are the system check's, times 0.75 under emulation.
  Results: D:\STELMAR-Gate\fit-fallback\results.jsonl (a finished row is skipped, so a stopped run resumes); evidence
  docs\evidence\phase-5\fit-and-fallback.txt, written from those results. Holds the gate lock (Invoke-WbGate).
#>
[CmdletBinding()]
param(
  [string]$Work = 'D:\STELMAR-Gate\fit-fallback',
  [string]$ExpDir = 'D:\wbt-src\exp\src-tauri\binaries',
  [string]$Python,
  [int]$TimeoutMin = 45,
  [string]$Only,
  [switch]$ReportOnly
)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo = (Resolve-Path (Join-Path $here '..\..')).Path
if (-not $Python) { $Python = Join-Path $repo 'eval\.venv\Scripts\python.exe' }
$Emulate = Join-Path $repo 'target\release\wb-emulate.exe'
Import-Module (Join-Path $repo 'scripts\lib\PsEnv.psm1') -Force
Set-WbCleanPSModulePath
Set-WbInvariantCulture
Import-Module (Join-Path $repo 'scripts\lib\GateRun.psm1') -Force
$inv = [Globalization.CultureInfo]::InvariantCulture
$triple = 'x86_64-pc-windows-msvc'
$minPath = "$env:SystemRoot\System32;$env:SystemRoot"
$results = Join-Path $Work 'results.jsonl'
New-Item -ItemType Directory -Force -Path $Work | Out-Null

function Get-Events($Lines) { @($Lines | Where-Object { $_ -match '^\{' } | ForEach-Object { try { $_ | ConvertFrom-Json } catch { $null } } | Where-Object { $_ }) }

function Write-Evidence {
  if (-not (Test-Path $results)) { return }
  $rows = @(Get-Content $results -Encoding UTF8 | ForEach-Object { $_ | ConvertFrom-Json })
  $L = @('Hinglish model: graphics memory fit and the day fallbacks (Phase 5, Hinglish rollout, steps 3 and 4)',
         "date: $($rows[0].when) to $($rows[-1].when) (IST); machine: Legion (RTX 4070 Laptop 8 GB, driver $($rows[0].driver)); written by scripts/gates/fit-and-fallback.ps1",
         'method: wb-engine batch over a 120 s English + Hindi clip (FLEURS test utterances alternating with 0.8 s of silence; 60 s for the Hinglish rows under emulation), default rolling settings (beam 5, temperature fallback, 15 s chunks, sentence split). RTF = decode seconds / audio seconds from the engine done event; load time is not in it.',
         'bars (system check): rolling RTF <= 1.00, times 0.75 under emulation = 0.75; the night pass fits when RTF x 3 h <= 12 h (RTF <= 4.00). Emulation is optimistic for CPU and pessimistic for the integrated GPU (the UHD rows are a floor).',
         '', '== Part V: video memory on the RTX 4070 (CUDA engine, 8 CPU threads, native; nvidia-smi memory.used peak minus idle, sampled every 200 ms)')
  $L += 'models loaded | peak video memory MiB (above idle) | idle MiB | RTF | engine sha256 (first 16)'
  foreach ($r in @($rows | Where-Object { $_.part -eq 'V' })) {
    $L += "$($r.label) | $($r.vram_peak_mib) | $($r.vram_idle_mib) | $(if ($r.status -eq 'ok') { $r.rtf.ToString('F3', $inv) } else { $r.status }) | $($r.engine_sha)"
  }
  $L += ''
  $L += '== Part R: rolling RTF under emulation (rule (a) = today, rule (c) = both ways, Hinglish = rule (d2))'
  $L += 'profile | device | rule | models | threads | audio s | RTF | bar | meets bar | night hours for 3 h of audio | chunks | detections | both ways | peak working set GiB | engine sha256 (first 16)'
  foreach ($r in @($rows | Where-Object { $_.part -eq 'R' })) {
    if ($r.status -ne 'ok') { $L += "$($r.profile) | $($r.device) | $($r.rule) | $($r.label) | $($r.threads) | | $($r.status) | | | | | | | | $($r.engine_sha)"; continue }
    $bar = 0.75
    $L += "$($r.profile) | $($r.device) | $($r.rule) | $($r.label) | $($r.threads) | $($r.audio_s.ToString('F0', $inv)) | $($r.rtf.ToString('F3', $inv)) | $($bar.ToString('F2', $inv)) | $(if ($r.rtf -le $bar) { 'yes' } else { 'no' }) | $(($r.rtf * 3).ToString('F1', $inv)) | $($r.chunks) | $($r.detections) | $($r.both_ways) | $(($r.peak_ws / 1GB).ToString('F2', $inv)) | $($r.engine_sha)"
  }
  $L += ''
  $L += '== Part D: the cost of a language detector on the processor (emu-mid, CPU avx2, 12 threads, same 129 s clip: detection on = English + Hindi, off = Hindi alone)'
  $L += 'rule | models | RTF | detections | peak working set GiB | engine sha256 (first 16)'
  foreach ($r in @($rows | Where-Object { $_.part -eq 'D' })) {
    $L += "$($r.rule) | $($r.label) | $(if ($r.status -eq 'ok') { $r.rtf.ToString('F3', $inv) } else { $r.status }) | $($r.detections) | $(($r.peak_ws / 1GB).ToString('F2', $inv)) | $($r.engine_sha)"
  }
  $L += ''
  $L += '== Part E: the language detector alone (wb-engine detect, no decoding; 129 s clip; CPU avx2; seconds include the model load)'
  $L += 'profile | models | threads | RTF | elapsed s | chunks detected | peak working set GiB'
  foreach ($r in @($rows | Where-Object { $_.part -eq 'E' })) {
    $L += "$($r.profile) | $($r.label) | $($r.threads) | $(if ($r.status -eq 'ok') { $r.rtf.ToString('F3', $inv) } else { $r.status }) | $(if ($r.status -eq 'ok') { $r.elapsed_s.ToString('F1', $inv) } else { '-' }) | $($r.detections) | $(($r.peak_ws / 1GB).ToString('F2', $inv))"
  }
  $L += ''
  $L += 'the experiment engines (rule (c)) are development builds from commit f1d9251; the rule (a) rows on the same engines use no experiment switch and equal the release engine; the Hinglish rows use the release engines in src-tauri\binaries.'
  New-Item -ItemType Directory -Force -Path (Join-Path $repo 'docs\evidence\phase-5') | Out-Null
  $out = Join-Path $repo 'docs\evidence\phase-5\fit-and-fallback.txt'
  [IO.File]::WriteAllText($out, (($L -join "`r`n") + "`r`n"), (New-Object Text.UTF8Encoding($false)))
  Write-Host "evidence: $out"
}
if ($ReportOnly) { Write-Evidence; return }

# ---------------------------------------------------------------- inputs
foreach ($f in @('models\manifest.local.json', 'models\ggml-silero-v6.2.0.bin')) { if (-not (Test-Path (Join-Path $repo $f))) { throw "missing: $f" } }
foreach ($s in @(120, 60)) {
  $wav = Join-Path $Work "bilingual-$s.wav"
  if (-not (Test-Path $wav)) {
    $mk = Invoke-WbNative -FilePath $Python -ArgumentList @('-X', 'utf8', '-I', (Join-Path $here 'make_bilingual.py'), '--out', $wav, '--seconds', "$s")
    if ($mk.ExitCode -ne 0) { throw "make_bilingual failed: $($mk.Output -join ' ') $($mk.Error -join ' ')" }
    Write-Host ($mk.Output -join ' ')
  }
}
function Copy-Engine([string]$Src, [string]$Name, [switch]$Cuda) {
  if (-not (Test-Path $Src)) { throw "engine missing: $Src" }
  $dir = Join-Path $Work $Name
  New-Item -ItemType Directory -Force -Path $dir | Out-Null
  $exe = Join-Path $dir "$Name.exe"
  Copy-Item $Src $exe -Force
  if ($Cuda -and $env:CUDA_PATH) {
    foreach ($b in @((Join-Path $env:CUDA_PATH 'bin'), (Join-Path $env:CUDA_PATH 'bin\x64'))) {
      if (Test-Path $b) { Get-ChildItem $b -Filter '*.dll' -ErrorAction SilentlyContinue | Where-Object { $_.Name -match '^(cudart|cublas)' } | ForEach-Object { Copy-Item $_.FullName $dir -Force } }
    }
  }
  @{ Exe = $exe; Name = $Name; Sha = (Get-FileHash $exe -Algorithm SHA256).Hash.ToLowerInvariant().Substring(0, 16) }
}
$bin = Join-Path $repo 'src-tauri\binaries'
$eng = @{
  cuda       = Copy-Engine (Join-Path $bin "wb-engine-cuda-$triple.exe") 'wb-engine-fit-cuda' -Cuda
  avx2       = Copy-Engine (Join-Path $bin "wb-engine-avx2-$triple.exe") 'wb-engine-fit-avx2'
  vulkan     = Copy-Engine (Join-Path $bin "wb-engine-vulkan-$triple.exe") 'wb-engine-fit-vulkan'
}
# The rule (c) rows need development engines built from commit f1d9251 (`scripts/build-engines.ps1 -LangExperiments` in a
# worktree of that commit); the experiment was dropped on 26 Nov 2026 and those engines were removed. The saved rows stay.
foreach ($x in @(@('avx2-exp', 'avx2'), @('vulkan-exp', 'vulkan'))) {
  $src = Join-Path $ExpDir "wb-engine-$($x[1])-lang-experiments-$triple.exe"
  if (Test-Path $src) { $eng[$x[0]] = Copy-Engine $src "wb-engine-fit-$($x[1])-exp" }
}
$driver = (& nvidia-smi --query-gpu=driver_version --format=csv,noheader 2>$null | Select-Object -First 1)

function Get-VramMiB { [int]((& nvidia-smi --query-gpu=memory.used --format=csv,noheader,nounits 2>$null | Select-Object -First 1).Trim()) }

# Runs one engine process (optionally under wb-emulate) and samples memory; returns the parsed run.
function Invoke-Run([string]$Profile, [hashtable]$E, [string[]]$EngineArgs, [hashtable]$Env = @{}, [switch]$Vram) {
  $o = Join-Path $Work 'run.out'; $er = Join-Path $Work 'run.err'
  $argList = $(if ($Profile -eq 'native') { $EngineArgs } else { @('--profile', $Profile, '--', ('"' + $E.Exe + '"')) + $EngineArgs })
  $file = $(if ($Profile -eq 'native') { $E.Exe } else { $Emulate })
  $saved = $env:PATH; $savedVars = @{}
  $env:PATH = $minPath
  foreach ($k in $Env.Keys) { $savedVars[$k] = [Environment]::GetEnvironmentVariable($k); [Environment]::SetEnvironmentVariable($k, $Env[$k]) }
  try { $p = Start-Process -FilePath $file -ArgumentList $argList -PassThru -WindowStyle Hidden -RedirectStandardOutput $o -RedirectStandardError $er }
  finally { $env:PATH = $saved; foreach ($k in $Env.Keys) { [Environment]::SetEnvironmentVariable($k, $savedVars[$k]) } }
  $null = $p.Handle
  $peakWs = 0L; $vramPeak = 0; $timedOut = $false
  $watch = [Diagnostics.Stopwatch]::StartNew()
  while (-not $p.HasExited) {
    foreach ($q in @(Get-Process -Name $E.Name -ErrorAction SilentlyContinue)) { try { if ($q.PeakWorkingSet64 -gt $peakWs) { $peakWs = $q.PeakWorkingSet64 } } catch { } }
    if ($Vram) { $v = Get-VramMiB; if ($v -gt $vramPeak) { $vramPeak = $v } }
    if ($watch.Elapsed.TotalMinutes -ge $TimeoutMin) {
      $timedOut = $true
      Get-Process -Name $E.Name -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
      try { $p.Kill() } catch { }
      break
    }
    Start-Sleep -Milliseconds 200
  }
  $p.WaitForExit()
  [pscustomobject]@{ Exit = $(if ($timedOut) { -2 } else { $p.ExitCode }); TimedOut = $timedOut; PeakWs = $peakWs; VramPeak = $vramPeak
    Out = @(Get-Content $o -Encoding UTF8 -ErrorAction SilentlyContinue); Err = @(Get-Content $er -Encoding UTF8 -ErrorAction SilentlyContinue) }
}

function Test-Done([string]$Key) {
  if (-not (Test-Path $results)) { return $false }
  [bool](Select-String -Path $results -SimpleMatch "`"key`":`"$Key`"" -Quiet)
}

function Save-Row($Rec) { ($Rec | ConvertTo-Json -Compress -Depth 5) | Add-Content -Path $results -Encoding ascii }

function Invoke-Batch([string]$Key, [string]$Part, [string]$Profile, [string]$Engine, [string]$Device, [string]$Rule, [string]$Label, [string]$Models, [int]$Threads, [string]$Clip, [hashtable]$Env = @{}, [switch]$Vram, [string]$Langs = 'en,hi') {
  if ($Only -and $Key -notlike "*$Only*") { return }
  if (Test-Done $Key) { Write-Host "skip (done): $Key"; return }
  $E = $eng[$Engine]
  $wav = Join-Path $Work $Clip
  $audioS = ((Get-Item $wav).Length - 44) / 2 / 16000
  $a = @('batch', '--file', ('"' + $wav + '"'), '--models', $Models, '--langs', $Langs, '--device', $Device, '--threads', "$Threads", '--models-dir', ('"' + (Join-Path $repo 'models') + '"'))
  Write-Host "$(Get-Date -Format 'HH:mm:ss') $Key"
  $idle = $(if ($Vram) { Get-VramMiB } else { 0 })
  $r = Invoke-Run $Profile $E $a $Env -Vram:$Vram
  $ev = Get-Events $r.Out
  $done = $ev | Where-Object { $_.event -eq 'done' } | Select-Object -First 1
  $err = $ev | Where-Object { $_.event -eq 'error' } | Select-Object -First 1
  $stats = @($ev | Where-Object { $_.event -eq 'stats' })
  $rec = [ordered]@{ key = $Key; part = $Part; profile = $Profile; device = $(if ($Device -eq 'cpu') { 'CPU avx2' } elseif ($Profile -eq 'native') { 'RTX 4070 (CUDA)' } else { 'Intel UHD (Vulkan)' })
    rule = $Rule; label = $Label; threads = $Threads; engine = $Engine; engine_sha = $E.Sha; when = (Get-Date).ToString('yyyy-MM-dd HH:mm', $inv); driver = $driver
    exit = $r.Exit; peak_ws = $r.PeakWs; vram_idle_mib = $idle; vram_peak_mib = $(if ($Vram) { $r.VramPeak - $idle } else { $null }) }
  if ($done -and $r.Exit -eq 0) {
    $rec.status = 'ok'; $rec.audio_s = $done.audio_ms / 1000.0; $rec.rtf = $done.elapsed_s / ($done.audio_ms / 1000.0); $rec.chunks = $done.chunks
    $rec.detections = [int](($stats | Measure-Object -Property detections -Sum).Sum); $rec.both_ways = [int](($stats | Measure-Object -Property both_ways -Sum).Sum)
  } else {
    $rec.status = $(if ($r.TimedOut) { 'timeout' } elseif ($err) { "error: $($err.code)" } else { "exit $($r.Exit): " + (($r.Err | Select-Object -Last 2) -join ' ') })
  }
  Save-Row $rec
}

function Invoke-Detect([string]$Key, [string]$Profile, [string]$Label, [string]$Model, [int]$Threads, [string]$Clip) {
  # `wb-engine detect`: the language of every chunk of a clip and no decoding; its done event holds the seconds from model load to the end.
  if ($Only -and $Key -notlike "*$Only*") { return }
  if (Test-Done $Key) { Write-Host "skip (done): $Key"; return }
  $E = $eng['avx2']
  $wav = Join-Path $Work $Clip
  $audioS = ((Get-Item $wav).Length - 44) / 2 / 16000
  $outFile = Join-Path $Work 'detect.jsonl'
  $a = @('detect', '--file', ('"' + $wav + '"'), '--models', "en=$Model,hi=$Model", '--langs', 'en,hi', '--device', 'cpu', '--threads', "$Threads", '--models-dir', ('"' + (Join-Path $repo 'models') + '"'), '--out', ('"' + $outFile + '"'))
  Write-Host "$(Get-Date -Format 'HH:mm:ss') $Key"
  $r = Invoke-Run $Profile $E $a
  $done = Get-Events $r.Out | Where-Object { $_.event -eq 'done' } | Select-Object -First 1
  $rec = [ordered]@{ key = $Key; part = 'E'; profile = $Profile; device = 'CPU avx2'; rule = 'detect only'; label = $Label; threads = $Threads; engine = 'avx2'; engine_sha = $E.Sha
    when = (Get-Date).ToString('yyyy-MM-dd HH:mm', $inv); exit = $r.Exit; peak_ws = $r.PeakWs }
  if ($done -and $r.Exit -eq 0) {
    $rec.status = 'ok'; $rec.audio_s = $audioS; $rec.elapsed_s = $done.elapsed_s; $rec.chunks = $done.chunks; $rec.rtf = $done.elapsed_s / $audioS
    $rec.detections = $done.chunks; $rec.both_ways = 0
  } else { $rec.status = "exit $($r.Exit): " + (($r.Err | Select-Object -Last 2) -join ' ') }
  Save-Row $rec
}

function Get-GpuIndex([string]$Profile, [string]$Engine, [string]$Match) {
  $r = Invoke-Run $Profile $eng[$Engine] @('info')
  $info = (Get-Events $r.Out | Where-Object { $_.event -eq 'info' } | Select-Object -First 1)
  if (-not $info) { return $null }
  $want = $(if ($Engine -like 'cuda*') { 'CUDA' } else { 'Vulkan' })
  $d = @($info.devices | Where-Object { $_.backend -eq $want -and $_.kind -ne 'cpu' -and (("$($_.description) $($_.name) $($_.adapter_name)") -match $Match) }) | Select-Object -First 1
  if ($d) { [int]$d.index } else { $null }
}

function Invoke-All {
  # ---- Part V: video memory on the RTX
  $cudaIdx = Get-GpuIndex 'native' 'cuda' 'NVIDIA|RTX'
  if ($null -eq $cudaIdx) { throw 'no CUDA device found' }
  $turbo = 'large-v3-turbo-q5_0'
  $combos = @(
    @{ id = 'turbo'; label = "$turbo alone (reference)"; models = "en=$turbo,hi=$turbo" },
    @{ id = 'f16'; label = 'hinglish-f16 alone'; models = 'en=hinglish-f16,hi=hinglish-f16' },
    @{ id = 'q8'; label = 'hinglish-q8_0 alone'; models = 'en=hinglish-q8_0,hi=hinglish-q8_0' },
    @{ id = 'f16+turbo'; label = "hinglish-f16 + $turbo loaded together"; models = "en=$turbo,hi=hinglish-f16" },
    @{ id = 'q8+turbo'; label = "hinglish-q8_0 + $turbo loaded together"; models = "en=$turbo,hi=hinglish-q8_0" },
    @{ id = 'q8+small'; label = 'hinglish-q8_0 + small-q5_0 loaded together (26 Nov 2026, step 4: the small detector)'; models = 'en=small-q5_0,hi=hinglish-q8_0' }
  )
  foreach ($c in $combos) { Invoke-Batch "V|$($c.id)" 'V' 'native' 'cuda' "gpu:$cudaIdx" 'd2' $c.label $c.models 8 'bilingual-120.wav' -Vram }

  # ---- Part R: rule (a) vs (c) and the Hinglish model under emulation
  $profiles = @(@{ p = 'emu-low'; th = 4; gth = 2 }, @{ p = 'emu-mid'; th = 12; gth = 6 })
  foreach ($pr in $profiles) {
    $uhd = Get-GpuIndex $pr.p 'vulkan' 'Intel|UHD'
    $rows = @(
      @{ dev = 'cpu'; engine = 'avx2'; exp = 'avx2-exp'; th = $pr.th; dname = 'cpu' }
    )
    if ($null -ne $uhd) { $rows += @{ dev = "gpu:$uhd"; engine = 'vulkan'; exp = 'vulkan-exp'; th = $pr.gth; dname = 'uhd' } }
    foreach ($d in $rows) {
      $m = 'en=medium,hi=large-v3-turbo-q5_0'
      Invoke-Batch "R|$($pr.p)|$($d.dname)|a" 'R' $pr.p $d.exp $d.dev 'a' 'medium (en) + turbo-q5_0 (hi)' $m $d.th 'bilingual-120.wav'
      Invoke-Batch "R|$($pr.p)|$($d.dname)|c" 'R' $pr.p $d.exp $d.dev 'c' 'medium (en) + turbo-q5_0 (hi)' $m $d.th 'bilingual-120.wav' @{ WB_LANG_EXPERIMENT = 'c' }
      Invoke-Batch "R|$($pr.p)|$($d.dname)|d2-f16" 'R' $pr.p $d.engine $d.dev 'd2' 'hinglish-f16 for every chunk' 'en=hinglish-f16,hi=hinglish-f16' $d.th 'bilingual-60.wav'
    }
  }

  # ---- Part D (26 Nov 2026, step 4): what a language detector costs on the processor. The engine detects the language of
  # every chunk with the model of the first language (a full encoder pass on a 30 s window) when the session has two
  # languages and not at all when it has one: the same clip decoded as English + Hindi (detection on) and as Hindi alone
  # (detection off) with the same model differs by the detection and the English decode.
  $mid = @{ p = 'emu-mid'; th = 12 }
  foreach ($det in @(@{ id = 'large-v3-turbo-q5_0'; name = 'turbo q5_0' }, @{ id = 'small-q5_0'; name = 'small q5_0' })) {
    Invoke-Batch "D|emu-mid|$($det.id)|detect-on" 'D' $mid.p 'avx2' 'cpu' 'detect on' "$($det.name) en + hi, detection on" "en=$($det.id),hi=$($det.id)" $mid.th 'bilingual-120.wav'
    Invoke-Batch "D|emu-mid|$($det.id)|detect-off" 'D' $mid.p 'avx2' 'cpu' 'detect off' "$($det.name) hi alone, no detection" "hi=$($det.id)" $mid.th 'bilingual-120.wav' -Langs 'hi'
  }
  # ---- Part E: the detector alone (no decoding), same clip, emu-mid CPU 12 threads, and the same on emu-low CPU 4 threads.
  foreach ($pr in @(@{ p = 'emu-mid'; th = 12 }, @{ p = 'emu-low'; th = 4 })) {
    foreach ($det in @(@{ id = 'large-v3-turbo-q5_0'; name = 'turbo q5_0' }, @{ id = 'small-q5_0'; name = 'small q5_0' })) {
      Invoke-Detect "E|$($pr.p)|$($det.id)" $pr.p "$($det.name) detector, no decoding" $det.id $pr.th 'bilingual-120.wav'
    }
  }
}

$requestFlag = 'D:\Projects\stelmar-gate-request.flag'
$ownsFlag = -not (Test-Path $requestFlag)
if ($ownsFlag) { Set-Content -LiteralPath $requestFlag -Value "requested by=fit-and-fallback`r`nstarted=$((Get-Date).ToString('o'))`r`npid=$PID" -Encoding ascii }
try {
  Invoke-WbGate -Name 'fit and fallback' -Body { Invoke-All }
  if (-not $Only) { Write-Evidence }
} finally {
  if ($ownsFlag) { Remove-Item -LiteralPath $requestFlag -Force -ErrorAction SilentlyContinue }
}
Write-Host 'fit and fallback done'
