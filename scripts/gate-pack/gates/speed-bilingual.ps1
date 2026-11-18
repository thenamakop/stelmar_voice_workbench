# Gate speed-bilingual (information only, no bar): how much does per-chunk language detection slow real queue
# work on THIS laptop? The system check benches one language with no detection; the queue of a session with
# two languages detects the language of every chunk. The gate runs the engine's own `batch` command with the
# request the queue builds, on the cell this laptop's quick check chose (its engine build, device, threads and
# rolling model per language, from the capability.json that gate kept), for rolling and for night:
#   fixed hi on bench-hi.wav, no detection  /  en+hi on bench-codeswitch.wav, detection on every chunk.
# Night uses large-v3-turbo q5_0 for English and large-v3 q5_0 for Hindi, as the queue's night engine loads them.
# -Quick: each measurement once (about 8 minutes). Full: twice, both reported with the median (about 15 minutes).
# No engine change, no new bar: the result is INFO. Windows PowerShell 5.1, ASCII only.
param([string]$CtxFile, [string]$ResultFile)
$ErrorActionPreference = 'Stop'
$lib = Join-Path $PSScriptRoot '..\..\lib'
Import-Module (Join-Path $lib 'PackCore.psm1') -Force
Import-Module (Join-Path $lib 'PackGate.psm1') -Force
$ctx = Read-PackCtx -CtxFile $CtxFile
Initialize-PackGate -Ctx $ctx
Import-Module (Join-Path $lib 'GateRun.psm1') -Force
Import-Module (Join-Path $lib 'Redact.psm1') -Force
Import-Module (Join-Path $lib 'SpeedTable.psm1') -Force
Import-Module (Join-Path $lib 'SpeedBilingual.psm1') -Force
$paths = Get-PackPaths -Ctx $ctx

Invoke-PackGateBody -Ctx $ctx -Id 'speed-bilingual' -ResultFile $ResultFile -Body {
  $quick = [bool]$ctx.quick
  $profile = [string]$ctx.profile
  $gdir = [string]$ctx.gate_dir
  $work = Join-Path $gdir 'work'
  New-Item -ItemType Directory -Force -Path $work | Out-Null
  $triple = 'x86_64-pc-windows-msvc'
  $thr = 'information only: RTF with and without per-chunk language detection, rolling and night, on the cell this laptop''s quick check chose; no bar'

  $capPath = Join-Path ([string]$ctx.work_dir) 'gates\quick-check\capability.json'
  $cap = Read-PackJson -Path $capPath
  if ($null -eq $cap) { return (New-PackResult -Id 'speed-bilingual' -Status 'INVALID' -Headline 'the quick check has not kept a capability.json on this laptop yet (run quick-check first: it chooses the cells this gate measures)' -Threshold $thr -Quick $quick) }
  if ($profile -ne 'laptop' -and (-not $paths.Emulate -or -not (Test-Path -LiteralPath $paths.Emulate))) { throw "the emulation profile $profile needs wb-emulate; it is not available (dry run on the Legion only)" }

  $hi = Get-BilingualCell -Capability $cap -Lang 'hi'
  $en = Get-BilingualCell -Capability $cap -Lang 'en'
  if ($null -eq $hi) {
    $why = ''
    try { $why = (@($cap.decision.tier_reasons) -join ' ') } catch { }
    return (New-PackResult -Id 'speed-bilingual' -Status 'INVALID' -Headline ('this laptop''s quick check chose no rolling model for Hindi, so there is no cell to measure. ' + $why).Trim() -Threshold $thr -Quick $quick)
  }
  $notes = New-Object System.Collections.Generic.List[string]
  if ($null -eq $en) { $notes.Add('the quick check chose no rolling model for English: the en+hi rolling measurement cannot be made') }
  elseif ($en.Variant -ne $hi.Variant -or $en.DeviceKind -ne $hi.DeviceKind -or $en.GpuIndex -ne $hi.GpuIndex -or $en.Threads -ne $hi.Threads) {
    $notes.Add(('the English cell of the quick check ran on {0}; all measurements here use the Hindi cell''s engine, device and threads ({1}), as one engine holds both models' -f (Format-BilingualWhere -Hi $en -En $null), (Format-BilingualWhere -Hi $hi -En $null)))
  }
  try { if ([string]$cap.decision.config.config.kind -eq 'hybrid') { $notes.Add('this laptop runs CPU and graphics together on different units; this gate measures the Hindi rolling cell''s single device') } } catch { }

  # Models and clips: every id the plan needs must be a file of the pack.
  $modelPaths = @{}
  foreach ($m in @(Get-SpeedModels)) { $f = Join-Path $paths.Models $m.File; if (Test-Path -LiteralPath $f) { $modelPaths[$m.Id] = $f } }
  $vad = Join-Path $paths.Models 'ggml-silero-v6.2.0.bin'
  if (-not (Test-Path -LiteralPath $vad)) { throw "the VAD model is missing from the pack: $vad" }
  $exeSrc = Join-Path $paths.Engines ("wb-engine-{0}-{1}.exe" -f $hi.Variant, $triple)
  if (-not (Test-Path -LiteralPath $exeSrc)) { return (New-PackResult -Id 'speed-bilingual' -Status 'INVALID' -Headline ('the engine build the quick check chose ({0}) is not in this pack' -f $hi.Variant) -Threshold $thr -Quick $quick) }
  $engDir = Join-Path $work 'engine'
  New-Item -ItemType Directory -Force -Path $engDir | Out-Null
  $procName = 'wb-engine-bilingual'
  $exe = Join-Path $engDir ($procName + '.exe')
  Copy-Item -LiteralPath $exeSrc -Destination $exe -Force

  $plan = @(Get-BilingualPlan -Hi $hi -En $en -Quick $quick)
  $notMeasured = New-Object System.Collections.Generic.List[string]
  $runnable = New-Object System.Collections.Generic.List[object]
  foreach ($m in $plan) {
    $missing = @($m.Models.Values | Where-Object { -not $modelPaths.ContainsKey([string]$_) })
    $clip = Join-Path $paths.BenchClips $m.Clip
    if ($missing.Count -gt 0) { $notMeasured.Add(('{0}: model {1} is not in the pack' -f $m.Key, ($missing -join ', '))); continue }
    if (-not (Test-Path -LiteralPath $clip)) { $notMeasured.Add(('{0}: clip {1} is not in the pack' -f $m.Key, $m.Clip)); continue }
    $runnable.Add($m)
  }
  Write-PackGateLog -Ctx $ctx -Message ('speed-bilingual: {0} measurements planned, {1} repeat(s) each; cell: {2}' -f $runnable.Count, $(if ($quick) { 1 } else { 2 }), (Format-BilingualWhere -Hi $hi -En $en))

  $resFile = Join-Path $gdir 'runs.jsonl'
  $runs = @{}
  foreach ($m in $runnable) { $runs[$m.Key] = New-Object System.Collections.Generic.List[object] }
  $failed = @{}
  $reps = $(if ($quick) { 1 } else { 2 })
  Invoke-WbGate -Name 'speed-bilingual' -Body {
    for ($rep = 1; $rep -le $reps; $rep++) {
      foreach ($m in $runnable) {
        if ($failed.ContainsKey($m.Key)) { continue }
        $req = New-BilingualRequest -Measurement $m -Cell $hi -ClipPath (Join-Path $paths.BenchClips $m.Clip) -ModelPaths $modelPaths -VadPath $vad
        $reqFile = Join-Path $work 'request.json'
        [IO.File]::WriteAllText($reqFile, ($req | ConvertTo-Json -Depth 8), (New-Object Text.UTF8Encoding($false)))
        Write-PackGateLog -Ctx $ctx -Message ('{0} run {1}/{2}' -f $m.Key, $rep, $reps)
        $r = Invoke-SpeedEngine -Exe $exe -ProcName $procName -EngineArgs @('batch', '--request', ('"' + $reqFile + '"')) -TimeoutMin 30 -WorkDir $work -Emulate $paths.Emulate -Profile $profile
        $st = Get-BilingualRunStats -Events @(Read-SpeedEvents $r.Out)
        if ($r.Exit -ne 0 -and $st.ok) { $st.ok = $false; $st.error = ('exit code {0}' -f $r.Exit) }
        if (-not $st.ok -and -not $st.error) {
          $tail = (@($r.Err | Where-Object { "$_".Trim() } | Select-Object -Last 2) -join ' ') -replace '[\r\n]+', ' '
          $st.error = $(if ($r.TimedOut) { 'timed out after 30 minutes' } elseif ($tail) { 'exit {0}: {1}' -f $r.Exit, $tail } else { 'exit {0}, no stats event' -f $r.Exit })
        }
        $runs[$m.Key].Add([pscustomobject]@{ rep = $rep; ok = $st.ok; error = $st.error; rtf = $st.rtf; decode_s = $st.decode_s; audio_ms = $st.audio_ms; chunks = $st.chunks; detections = $st.detections; both_ways = $st.both_ways
            load_s = $st.load_s; device_name = $st.device_name; segments = $st.segments; peak_ws = $r.PeakWs; wall_s = $r.Seconds; exit = $r.Exit; whisper = $st.whisper })
        if (-not $st.ok) { $failed[$m.Key] = $true; $notMeasured.Add(('{0}: {1}' -f $m.Key, $st.error)) }
        ($runs[$m.Key][$runs[$m.Key].Count - 1] | Add-Member -NotePropertyName key -NotePropertyValue $m.Key -PassThru | ConvertTo-Json -Compress) | Add-Content -LiteralPath $resFile -Encoding ASCII
      }
    }
  } | Out-Null

  # Per-measurement JSON, the evidence text and the result.
  $records = New-Object System.Collections.Generic.List[object]
  $evFiles = New-Object System.Collections.Generic.List[string]
  foreach ($m in $runnable) {
    $mr = @($runs[$m.Key].ToArray())
    $records.Add([pscustomobject]@{ key = $m.Key; label = $m.Label; models = (($m.Models.Keys | ForEach-Object { '{0}={1}' -f $_, $m.Models[$_] }) -join ', '); clip = $m.Clip; languages = ($m.Languages -join '+'); detect = $m.Detect; runs = $mr })
    $okRuns = @($mr | Where-Object { $_.ok })
    $j = [ordered]@{
      key = $m.Key; label = $m.Label; role = $m.Role; clip = $m.Clip; languages = @($m.Languages); detect_language = [bool]$m.Detect
      models = $m.Models; engine = [ordered]@{ variant = $hi.Variant; device = (Get-BilingualDevice -Cell $hi); threads = $hi.Threads }
      repeats_planned = $m.Repeats; median_rtf = $(if ($okRuns.Count) { Get-BilingualMedian -Values @($okRuns | ForEach-Object { [double]$_.rtf }) } else { $null })
      runs = @($mr)
    }
    $jp = Join-Path $gdir ('measurement-{0}.json' -f $m.Key)
    Write-PackJson -Path $jp -Value $j
    $evFiles.Add($jp)
  }
  # Plain arrays, not the generic lists: @() around a List makes the next call fail with "Argument types do not match" (found in the dry run of 18 Nov 2026).
  function RunsOf([string]$Key) { if ($runs.ContainsKey($Key)) { return , @($runs[$Key].ToArray()) } else { return , @() } }
  $rolling = Get-BilingualPart -FixedRuns (RunsOf 'rolling-fixed-hi') -DetectRuns (RunsOf 'rolling-detect-en-hi')
  $night = Get-BilingualPart -FixedRuns (RunsOf 'night-fixed-hi') -DetectRuns (RunsOf 'night-detect-en-hi')
  $lines = @(Get-BilingualReportLines -Records @($records.ToArray()) -Rolling $rolling -Night $night -Machine ([string]$ctx.machine) -Profile $profile -Quick $quick -Hi $hi -En $en -Notes @($notes.ToArray()))
  $evidence = Join-Path $gdir 'speed-bilingual.txt'
  Write-WbEvidence -Path $evidence -Lines $lines -MachineLabel ([string]$ctx.machine)
  $evFiles.Insert(0, $evidence)
  $evFiles.Add($resFile)

  $where = Format-BilingualWhere -Hi $hi -En $en
  $head = Format-BilingualHeadline -Rolling $rolling -Night $night -Where $where -NotMeasured @($notMeasured.ToArray() | Select-Object -Unique)
  $status = 'INFO'
  if (-not $rolling.Ok -and -not $night.Ok) { $status = 'INVALID' }
  $measured = [ordered]@{
    cell = [ordered]@{ variant = $hi.Variant; device = (Get-BilingualDevice -Cell $hi); threads = $hi.Threads; hi_model = $hi.Model; en_model = $(if ($en) { $en.Model } else { $null }); night_models = (Get-BilingualNightModels) }
    rolling = $rolling; night = $night
  }
  New-PackResult -Id 'speed-bilingual' -Status $status -Headline $head -Measured $measured -Threshold $thr -Evidence @($evFiles.ToArray()) -Notes @($notes.ToArray()) -Quick $quick
}
