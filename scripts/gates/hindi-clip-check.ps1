<#
.SYNOPSIS
  Hindi turbo on the Vulkan UHD under emu-low: is the gap between the Phase 2 speed table (RTF 0.734) and the
  quick check (0.83) the clip or the excerpt length? (owner, 13 Nov 2026, decision 2)

.DESCRIPTION
  Three clips, each timed three times, the three clips interleaved (a, b, c, a, b, c, a, b, c) so slow drift of
  the machine does not favour one:
    (a) the bundled Hindi bench clip (resources\bench\bench-hi.wav), first 30 s: what the quick check times
    (b) the same clip, the whole 60 s: what the full check times
    (c) the Phase 2 speed-table Hindi clip (D:\STELMAR-Gate\speed\clip-hi.wav, 60.2 s)
  Each run is one engine process: `wb-engine bench` (warm-up run, then one timed run), model large-v3-turbo q5_0,
  the Vulkan engine on the Intel UHD with 2 CPU threads, rolling settings, under `wb-emulate --profile emu-low`.
  That is exactly one cell of the quick check, so a run here is comparable with a quick-check cell. The script
  prints each run, the median of the three timed runs per clip (and of the warm-ups), and the arithmetic
  differences; it also prints whether (b) and (c) hold the same audio and how dense the clip is
  (scripts\gates\hindi_clip_density.py: characters of reference text per second, against every 60 s window of the
  FLEURS Hindi test set). It decides nothing: the reading of the numbers goes in a note that names its author.

  Timing-sensitive: holds the gate lock, refuses with overlay tools open or a build running. About 15 minutes.
  Nothing else may run. Evidence: docs\evidence\phase-3\hindi-clip-check.txt (through scripts\lib\Redact.psm1).
#>
[CmdletBinding()]
param(
  [string]$Work = 'D:\STELMAR-Gate\hindi-clip',
  [string]$OutDir,
  [string]$Emulate,
  [string]$Machine = 'Legion',
  [string]$EmuProfile = 'emu-low',
  [int]$Rounds = 3,
  [int]$CellTimeoutMin = 20
)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo = (Resolve-Path (Join-Path $here '..\..')).Path
if (-not $OutDir) { $OutDir = Join-Path $repo 'docs\evidence\phase-3' }
if (-not $Emulate) { $Emulate = Join-Path $repo 'target\release\wb-emulate.exe' }
Import-Module (Join-Path $repo 'scripts\lib\PsEnv.psm1') -Force
Set-WbCleanPSModulePath
Set-WbInvariantCulture
Import-Module (Join-Path $repo 'scripts\lib\GateRun.psm1') -Force
Import-Module (Join-Path $repo 'scripts\lib\Redact.psm1') -Force
$python = Join-Path $repo 'eval\.venv\Scripts\python.exe'
$inv = [Globalization.CultureInfo]::InvariantCulture
function F([double]$v, [int]$d = 3) { $v.ToString("F$d", $inv) }
$triple = 'x86_64-pc-windows-msvc'
$minPath = "$env:SystemRoot\System32;$env:SystemRoot"
$bundled = Join-Path $repo 'resources\bench\bench-hi.wav'
$phase2 = 'D:\STELMAR-Gate\speed\clip-hi.wav'
$modelPath = Join-Path $repo 'models\ggml-large-v3-turbo-q5_0.bin'
$vadPath = Join-Path $repo 'models\ggml-silero-v6.2.0.bin'
foreach ($f in @($bundled, $phase2, $modelPath, $vadPath, $Emulate, $python)) { if (-not (Test-Path -LiteralPath $f)) { throw "missing: $f" } }
New-Item -ItemType Directory -Force -Path $Work, $OutDir | Out-Null

# The three clips and the facts about them (audio is read here, never committed).
$prep = Join-Path $Work 'prep.py'
@'
import json, sys, wave, hashlib
bundled, phase2, out_a = sys.argv[1], sys.argv[2], sys.argv[3]
def rd(p):
    with wave.open(p) as w:
        assert w.getframerate() == 16000 and w.getnchannels() == 1 and w.getsampwidth() == 2
        return w.readframes(w.getnframes())
b, c = rd(bundled), rd(phase2)
n = min(len(b), len(c))
with wave.open(out_a, "wb") as o:
    o.setnchannels(1); o.setsampwidth(2); o.setframerate(16000)
    o.writeframes(b[: 30 * 16000 * 2])
print(json.dumps({
    "bundled_s": len(b) / 32000, "phase2_s": len(c) / 32000,
    "same_audio_in_the_common_length": b[:n] == c[:n],
    "common_s": n / 32000,
    "bundled_sha256": hashlib.sha256(b).hexdigest(), "phase2_sha256": hashlib.sha256(c).hexdigest(),
}))
'@ | Set-Content -LiteralPath $prep -Encoding ascii
$clipA = Join-Path $Work 'bundled-first-30s.wav'
$facts = (& $python -I $prep $bundled $phase2 $clipA) | ConvertFrom-Json
$dens = Join-Path $Work 'density.json'
& $python -X utf8 (Join-Path $here 'hindi_clip_density.py') $dens | Out-Null
$density = Get-Content -LiteralPath $dens -Raw | ConvertFrom-Json

# Engine copy under a unique name (the app's own binary, the Vulkan build).
$src = Join-Path $repo "src-tauri\binaries\wb-engine-vulkan-$triple.exe"
if (-not (Test-Path -LiteralPath $src)) { throw "engine not built: $src" }
$engDir = Join-Path $Work 'engine'
New-Item -ItemType Directory -Force -Path $engDir | Out-Null
$exe = Join-Path $engDir 'wb-engine-hindi-check.exe'
Copy-Item -LiteralPath $src -Destination $exe -Force
$engSha = (Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash.ToLowerInvariant()

function Invoke-Engine([string[]]$EngineArgs, [int]$TimeoutMin) {
  $o = Join-Path $Work 'cell.out'; $er = Join-Path $Work 'cell.err'
  $argList = @('--profile', $EmuProfile, '--', ('"' + $exe + '"')) + $EngineArgs
  $saved = $env:PATH
  $env:PATH = $minPath
  try { $p = Start-Process -FilePath $Emulate -ArgumentList $argList -PassThru -WindowStyle Hidden -RedirectStandardOutput $o -RedirectStandardError $er }
  finally { $env:PATH = $saved }
  $null = $p.Handle
  $watch = [Diagnostics.Stopwatch]::StartNew()
  $timedOut = $false
  while (-not $p.HasExited) {
    if ($watch.Elapsed.TotalMinutes -ge $TimeoutMin) {
      $timedOut = $true
      Get-Process -Name 'wb-engine-hindi-check' -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
      try { $p.Kill() } catch { }
      break
    }
    Start-Sleep -Milliseconds 250
  }
  $p.WaitForExit()
  [pscustomobject]@{ Exit = $(if ($timedOut) { -2 } else { $p.ExitCode }); Seconds = [math]::Round($watch.Elapsed.TotalSeconds, 1); Out = @(Get-Content $o -Encoding UTF8 -ErrorAction SilentlyContinue) }
}
function Get-Events($lines) { foreach ($l in $lines) { try { $l | ConvertFrom-Json } catch { } } }

$median = { param($v) $s = @($v | Sort-Object); if ($s.Count % 2) { $s[[int](($s.Count - 1) / 2)] } else { ($s[$s.Count / 2 - 1] + $s[$s.Count / 2]) / 2 } }

Invoke-WbGate -Name 'hindi-clip-check' -Body {
  # The Intel UHD's Vulkan index under the profile.
  $r = Invoke-Engine -EngineArgs @('info') -TimeoutMin 5
  $info = Get-Events $r.Out | Where-Object { $_.event -eq 'info' } | Select-Object -First 1
  $gpu = $null
  if ($info) {
    $d = @($info.devices | Where-Object { $_.backend -eq 'Vulkan' -and $_.kind -ne 'cpu' -and (("$($_.description) $($_.name) $($_.adapter_name)") -match 'Intel|UHD') }) | Select-Object -First 1
    if ($d) { $gpu = [int]$d.index }
  }
  if ($null -eq $gpu) { throw 'the Intel UHD was not found among the Vulkan devices under the profile' }
  $script:gpuIndex = $gpu
  $clips = [ordered]@{ 'a_bundled_30s' = $clipA; 'b_bundled_60s' = $bundled; 'c_phase2_60s' = $phase2 }
  $script:runs = @()
  for ($round = 1; $round -le $Rounds; $round++) {
    foreach ($k in $clips.Keys) {
      $req = [ordered]@{
        type = 'bench'
        model = @{ id = 'large-v3-turbo-q5_0'; path = $modelPath }
        vad_model = @{ id = 'silero-v6.2.0'; path = $vadPath }
        lang = 'hi'; clip = $clips[$k]; mode = 'rolling'; device = @{ kind = 'gpu'; index = $gpu }; threads = 2; runs = 1
      }
      $reqFile = Join-Path $Work 'request.json'
      [IO.File]::WriteAllText($reqFile, ($req | ConvertTo-Json -Depth 6), (New-Object Text.UTF8Encoding($false)))
      Write-Host "$(Get-Date -Format 'HH:mm:ss') round $round $k"
      $r = Invoke-Engine -EngineArgs @('bench', '--request', ('"' + $reqFile + '"')) -TimeoutMin $CellTimeoutMin
      $b = Get-Events $r.Out | Where-Object { $_.event -eq 'bench' } | Select-Object -First 1
      $row = [ordered]@{ round = $round; clip = $k; exit = $r.Exit; wall_s = $r.Seconds }
      if ($b -and $r.Exit -eq 0) {
        $row.rtf = [double]$b.rtf_median; $row.warmup_rtf = [double]$b.warmup_rtf; $row.clip_s = [double]$b.clip_seconds
        $row.load_s = [double]$b.load_s; $row.segments = [int]$b.segments; $row.truncated = [int]$b.truncated_chunks; $row.device = "$($b.device_name)"
      }
      $script:runs += [pscustomobject]$row
    }
  }
} | Out-Null

$lines = @()
$lines += 'Hindi turbo on the Vulkan UHD under emu-low: the clip against the excerpt length (decision 2 of 13 Nov 2026)'
$lines += "date: $(Get-Date -Format 'yyyy-MM-dd HH:mm') (IST on the Legion)"
$lines += "machine: $Machine, profile $EmuProfile (4 logical processors, 11 GiB job limit), Vulkan device index $script:gpuIndex ($(@($script:runs | Where-Object device | Select-Object -First 1).device))"
$lines += "command: powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\gates\hindi-clip-check.ps1"
$lines += "source: $((Invoke-WbNative -FilePath 'git' -ArgumentList @('-C', $repo, 'rev-parse', '--short', 'HEAD')).Output -join '')"
$lines += "engine: wb-engine vulkan build, sha256 $engSha; model large-v3-turbo q5_0; Silero VAD v6.2.0; 2 CPU threads; rolling settings; one process per run = a warm-up decode and one timed decode (the quick check's cell), RTF = decode time / clip length (load time apart)"
$lines += "order: $Rounds rounds, each a (bundled 30 s), b (bundled 60 s), c (Phase 2 clip 60.2 s), so drift of the machine touches all three alike"
$lines += ''
$lines += 'The clips'
$lines += "  bundled clip $(F $facts.bundled_s 2) s, Phase 2 clip $(F $facts.phase2_s 2) s; the same audio over the common $(F $facts.common_s 2) s: $($facts.same_audio_in_the_common_length)"
$lines += '  a is the first 30 s of the bundled clip; b is the bundled clip; c is the Phase 2 speed-table clip (a trailing utterance fragment longer)'
$h = $density.hi
$lines += "  density (characters of reference text without spaces per second of clip; every 60 s window of the FLEURS Hindi test set, $($h.windows) windows): bundled clip $(F $h.chars_per_clip_second.bundled 2), median $(F $h.chars_per_clip_second.median 2), 10th to 90th percentile $(F $h.chars_per_clip_second.p10 2) to $(F $h.chars_per_clip_second.p90 2), the bundled clip sits at the $(F $h.chars_per_clip_second.percentile_of_bundled 0)th percentile"
$lines += "  density per second of speech (frames above -45 dBFS): bundled $(F $h.chars_per_speech_second.bundled 2), median $(F $h.chars_per_speech_second.median 2), 10th to 90th $(F $h.chars_per_speech_second.p10 2) to $(F $h.chars_per_speech_second.p90 2), $(F $h.chars_per_speech_second.percentile_of_bundled 0)th percentile"
$lines += "  the first 30 s of the bundled clip alone (the quick-check excerpt): $(F $h.first_30s_chars_per_second 2) characters per second (approximate: the cut utterance counted in proportion)"
$lines += ''
$lines += 'The runs (round, clip: timed RTF, warm-up RTF, clip seconds, load s, segments, truncated chunks, wall s)'
foreach ($r in $script:runs) {
  if ($r.PSObject.Properties.Name -contains 'rtf') { $lines += ("  {0} {1,-14} RTF {2}  warm-up {3}  {4} s  load {5} s  segments {6}  truncated {7}  wall {8} s" -f $r.round, $r.clip, (F $r.rtf), (F $r.warmup_rtf), (F $r.clip_s 1), (F $r.load_s 1), $r.segments, $r.truncated, $r.wall_s) }
  else { $lines += ("  {0} {1,-14} FAILED exit {2} after {3} s" -f $r.round, $r.clip, $r.exit, $r.wall_s) }
}
$lines += ''
$lines += 'Medians of the three runs per clip (timed RTF; warm-up RTF; range of the timed runs)'
$med = @{}
foreach ($k in @('a_bundled_30s', 'b_bundled_60s', 'c_phase2_60s')) {
  $ok = @($script:runs | Where-Object { $_.clip -eq $k -and ($_.PSObject.Properties.Name -contains 'rtf') })
  if (-not $ok.Count) { $lines += "  $k : no run finished"; continue }
  $m = & $median ($ok | ForEach-Object { $_.rtf })
  $w = & $median ($ok | ForEach-Object { $_.warmup_rtf })
  $sorted = @($ok | ForEach-Object { $_.rtf } | Sort-Object)
  $med[$k] = $m
  $lines += ("  {0,-14} timed {1}  warm-up {2}  range {3} to {4}  ({5} run(s))" -f $k, (F $m), (F $w), (F $sorted[0]), (F $sorted[-1]), $ok.Count)
}
$lines += ''
$lines += 'Differences (arithmetic only; the reading is in a separate note that names its author)'
if ($med.ContainsKey('a_bundled_30s') -and $med.ContainsKey('b_bundled_60s')) {
  $d = ($med['a_bundled_30s'] / $med['b_bundled_60s'] - 1) * 100
  $lines += "  excerpt length, same clip: 30 s against 60 s: $((F $d 1))%  (a over b)"
}
if ($med.ContainsKey('b_bundled_60s') -and $med.ContainsKey('c_phase2_60s')) {
  $d = ($med['b_bundled_60s'] / $med['c_phase2_60s'] - 1) * 100
  $lines += "  clip, 60 s each: bundled against Phase 2: $((F $d 1))%  (b over c; the audio is the same over the common length)"
}
if ($med.ContainsKey('c_phase2_60s')) {
  $lines += "  against the Phase 2 speed-table figure (UHD, 2 threads, emu-low, turbo q5_0, Hindi): 0.734; c now $(F $med['c_phase2_60s']): $((F (($med['c_phase2_60s'] / 0.734 - 1) * 100) 1))%"
}
if ($med.ContainsKey('a_bundled_30s')) { $lines += "  emulated bar 0.75: a $(if ($med['a_bundled_30s'] -le 0.75) { 'meets' } else { 'misses' })" }
if ($med.ContainsKey('b_bundled_60s')) { $lines += "  emulated bar 0.75: b $(if ($med['b_bundled_60s'] -le 0.75) { 'meets' } else { 'misses' })" }
if ($med.ContainsKey('c_phase2_60s')) { $lines += "  emulated bar 0.75: c $(if ($med['c_phase2_60s'] -le 0.75) { 'meets' } else { 'misses' })" }
Write-WbEvidence -Path (Join-Path $OutDir 'hindi-clip-check.txt') -Lines $lines -MachineLabel $Machine
Write-Host "evidence: $(Join-Path $OutDir 'hindi-clip-check.txt')"
