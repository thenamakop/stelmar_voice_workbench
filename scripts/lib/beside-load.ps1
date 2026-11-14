# beside-load.ps1: the engines that run beside the recorder in live-batch-safety and bg-decode-info.
# Started by scripts\gates\recorder-gate.ps1 (-BesideScript) once the recording runs; stopped when the
# STOP file appears. Writes <OutDir>\result.json.
#   live engine : wb-engine live, 16 kHz mono f32 PCM fed in real time from a clip, looped (voice
#                 detection only until Phase 7 adds decoding), normal priority.
#   batch engine: wb-engine batch decoding a clip again and again with a real model, below-normal priority.
# Config (JSON): mode (live-batch | batch), engines_dir, models_dir, clip, live_variant, batch_variant,
# batch_device (cpu | gpu:N), batch_threads, live_threads, model, lang.
# Windows PowerShell 5.1, ASCII only.
param([Parameter(Mandatory = $true)] [string]$Config, [Parameter(Mandatory = $true)] [string]$StopFile, [Parameter(Mandatory = $true)] [string]$OutDir)
$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$cfg = Get-Content -LiteralPath $Config -Raw -Encoding UTF8 | ConvertFrom-Json
$inv = [Globalization.CultureInfo]::InvariantCulture
$lines = New-Object System.Collections.Generic.List[string]
function Note([string]$m) { $lines.Add(('{0} {1}' -f (Get-Date -Format 'HH:mm:ss'), $m)) }
$result = [ordered]@{ ok = $false; mode = [string]$cfg.mode; started = (Get-Date).ToString('o'); ended = $null; live = $null; batch = $null; lines = @() }

function Get-Engine([string]$Variant) {
  $p = Join-Path ([string]$cfg.engines_dir) ("wb-engine-$Variant-x86_64-pc-windows-msvc.exe")
  if (-not (Test-Path -LiteralPath $p)) { throw "engine $Variant is not in the pack" }
  $p
}

# 16-bit PCM WAV (16 kHz mono) to float32 little-endian bytes.
function Read-ClipAsF32([string]$Path) {
  $b = [IO.File]::ReadAllBytes($Path)
  $pos = 12
  $at = -1; $len = 0
  while ($pos + 8 -le $b.Length) {
    $id = [Text.Encoding]::ASCII.GetString($b, $pos, 4)
    $l = [int][BitConverter]::ToUInt32($b, $pos + 4)
    if ($id -eq 'data') { $at = $pos + 8; $len = [math]::Min($l, $b.Length - $at); break }
    $pos += 8 + $l + ($l % 2)
  }
  if ($at -lt 0) { throw "no audio in $Path" }
  $n = [int]($len / 2)
  $f = New-Object byte[] ($n * 4)
  for ($i = 0; $i -lt $n; $i++) { [BitConverter]::GetBytes([single]([BitConverter]::ToInt16($b, $at + 2 * $i) / 32768.0)).CopyTo($f, 4 * $i) }
  , $f
}

$live = $null
$liveOut = $null; $liveErr = $null
$batchProc = $null
$stop = { Test-Path -LiteralPath $StopFile }
try {
  if ([string]$cfg.mode -eq 'live-batch') {
    $exe = Get-Engine ([string]$cfg.live_variant)
    $vad = Join-Path ([string]$cfg.models_dir) 'ggml-silero-v6.2.0.bin'
    $req = [ordered]@{ models_by_lang = @{}; vad_model = @{ id = 'silero-v6.2.0'; path = $vad }; languages = @('en'); device = @{ kind = 'cpu' }; threads = [int]$cfg.live_threads }
    $reqFile = Join-Path $OutDir 'live-request.json'
    [IO.File]::WriteAllText($reqFile, ($req | ConvertTo-Json -Depth 5), (New-Object Text.UTF8Encoding($false)))
    $psi = New-Object Diagnostics.ProcessStartInfo
    $psi.FileName = $exe
    $psi.Arguments = 'live --request "' + $reqFile + '"'
    $psi.UseShellExecute = $false
    $psi.RedirectStandardInput = $true; $psi.RedirectStandardOutput = $true; $psi.RedirectStandardError = $true
    $psi.CreateNoWindow = $true
    $live = [Diagnostics.Process]::Start($psi)
    $liveOut = $live.StandardOutput.ReadToEndAsync()
    $liveErr = $live.StandardError.ReadToEndAsync()
    Note ("live engine started: wb-engine-{0} live (pid {1}), normal priority, voice detection only" -f $cfg.live_variant, $live.Id)
  }
  # The batch engine, in a loop, until the stop file appears.
  $runs = New-Object System.Collections.Generic.List[object]
  $batchExe = Get-Engine ([string]$cfg.batch_variant)
  $clip = [string]$cfg.clip
  $pcm = $null
  if ($live) { $pcm = Read-ClipAsF32 $clip }
  $block = 1600 * 4
  $pos = 0
  $sent = [int64]0
  $sw = [Diagnostics.Stopwatch]::StartNew()
  $nextBlockAt = 0.0
  $liveBroken = $false
  $batchOut = Join-Path $OutDir 'batch-out.jsonl'
  while (-not (& $stop)) {
    # Start a batch run when none is running and memory allows (3 GB must stay free).
    if ($null -eq $batchProc -or $batchProc.HasExited) {
      if ($batchProc) {
        $runs.Add([ordered]@{ exit = $batchProc.ExitCode; seconds = [math]::Round(((Get-Date) - $batchStart).TotalSeconds, 1) })
        $batchProc = $null
      }
      $freeGb = (Get-CimInstance Win32_OperatingSystem).FreePhysicalMemory / 1MB
      if ($freeGb -lt 4.0) { Note ("batch engine not started: only {0:n1} GB of RAM free (3 GB must stay free)" -f $freeGb); Start-Sleep -Seconds 5 }
      else {
        $args1 = @('batch', '--file', ('"' + $clip + '"'), '--models', ('{0}={1}' -f $cfg.lang, $cfg.model), '--langs', [string]$cfg.lang, '--out', ('"' + $batchOut + '"'), '--threads', [string]$cfg.batch_threads, '--device', [string]$cfg.batch_device, '--models-dir', ('"' + [string]$cfg.models_dir + '"'))
        $batchStart = Get-Date
        $batchProc = Start-Process -FilePath $batchExe -ArgumentList $args1 -PassThru -WindowStyle Hidden -RedirectStandardOutput (Join-Path $OutDir 'batch.out') -RedirectStandardError (Join-Path $OutDir 'batch.err')
        $null = $batchProc.Handle
        try { $batchProc.PriorityClass = [Diagnostics.ProcessPriorityClass]::BelowNormal } catch { Note "could not lower the batch engine's priority: $($_.Exception.Message)" }
        if ($runs.Count -eq 0) { Note ("batch engine started: wb-engine-{0} batch, model {1}, device {2}, {3} threads, below-normal priority" -f $cfg.batch_variant, $cfg.model, $cfg.batch_device, $cfg.batch_threads) }
      }
    }
    # Feed the live engine in real time: 100 ms of audio per block.
    if ($live) {
      while (-not $live.HasExited -and $sw.Elapsed.TotalSeconds -ge $nextBlockAt) {
        if ($pos + $block -gt $pcm.Length) { $pos = 0 }
        try { $live.StandardInput.BaseStream.Write($pcm, $pos, $block); $live.StandardInput.BaseStream.Flush() } catch { $liveBroken = $true; break }
        $pos += $block; $sent += $block; $nextBlockAt += 0.1
      }
      if ($live.HasExited -or $liveBroken) { Note ("live engine ended early (exit {0})" -f $live.ExitCode); break }
    }
    Start-Sleep -Milliseconds 20
  }
  # Stop: close the live engine's input, let it finish, and stop the batch engine.
  $liveAlive = $false
  $liveExit = $null
  $vadEvents = 0
  if ($live) {
    $liveAlive = (-not $live.HasExited -and -not $liveBroken)
    try { $live.StandardInput.Close() } catch { }
    if (-not $live.WaitForExit(30000)) { try { $live.Kill() } catch { } }
    $liveExit = $live.ExitCode
    $txt = ''
    try { $txt = $liveOut.Result } catch { }
    $vadEvents = @([regex]::Matches($txt, '"event"\s*:\s*"vad"')).Count
    Set-Content -LiteralPath (Join-Path $OutDir 'live.out') -Value $txt -Encoding UTF8
    $et = ''; try { $et = $liveErr.Result } catch { }
    Set-Content -LiteralPath (Join-Path $OutDir 'live.err') -Value $et -Encoding UTF8
    $result.live = [ordered]@{ alive_at_end = $liveAlive; exit = $liveExit; vad_events = $vadEvents; seconds_fed = [math]::Round($sent / 4 / 16000.0, 1) }
    Note ("live engine: alive at the end {0}; exit code {1}; {2} vad events; {3:n0} s of audio fed in real time" -f $liveAlive, $liveExit, $vadEvents, ($sent / 4 / 16000.0))
  }
  $finishedRuns = @($runs.ToArray())
  $unfinished = $null
  if ($batchProc -and -not $batchProc.HasExited) {
    $unfinished = [math]::Round(((Get-Date) - $batchStart).TotalSeconds, 1)
    try { $batchProc.Kill() } catch { }
    $null = $batchProc.WaitForExit(15000)
  } elseif ($batchProc) { $finishedRuns += [ordered]@{ exit = $batchProc.ExitCode; seconds = [math]::Round(((Get-Date) - $batchStart).TotalSeconds, 1) } }
  $badExits = @($finishedRuns | Where-Object { $_.exit -ne 0 })
  $result.batch = [ordered]@{ completed_runs = $finishedRuns.Count; bad_exits = $badExits.Count; running_when_stopped_s = $unfinished; run_seconds = @($finishedRuns | ForEach-Object { $_.seconds }) }
  Note ("batch engine: {0} runs finished ({1} with a bad exit code){2}" -f $finishedRuns.Count, $badExits.Count, $(if ($null -ne $unfinished) { "; one still running after $unfinished s when the recording ended" } else { '' }))
  $liveOk = ([string]$cfg.mode -ne 'live-batch') -or ($result.live -and $result.live.alive_at_end -and [int]$result.live.exit -eq 0)
  $batchOk = ($badExits.Count -eq 0) -and (($finishedRuns.Count -ge 1) -or ($null -ne $unfinished))
  $result.ok = [bool]($liveOk -and $batchOk)
  if (-not $liveOk) { Note 'the live engine did not stay up for the whole recording' }
  if (-not $batchOk) { Note 'the batch engine did not run cleanly beside the recorder' }
} catch {
  Note ('beside-load failed: ' + $_.Exception.Message)
  $result.ok = $false
} finally {
  foreach ($p in @($live, $batchProc)) { if ($p -and -not $p.HasExited) { try { $p.Kill() } catch { } } }
  $result.ended = (Get-Date).ToString('o')
  $result.lines = @($lines.ToArray())
  ($result | ConvertTo-Json -Depth 6) | Set-Content -LiteralPath (Join-Path $OutDir 'result.json') -Encoding UTF8
}
