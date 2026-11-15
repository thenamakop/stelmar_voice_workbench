<#
  SpeedTable.psm1: the speed tables of the gate pack, run on the laptop itself.

  Two parts share this code:
    igpu  every real Vulkan graphics adapter the engine lists (driver version in every row), five models,
          English and Hindi, and the code-switched clip for the Hinglish model;
    cpu   the avx2 engine at the machine's physical-core and logical-processor thread counts, the avx512
          engine only where the CPU has AVX-512, the same models and clips, and small near-live.
  Nothing is hard-coded to a class of machine: adapters and thread counts come from `wb-engine info` and
  the fingerprint of this laptop. The result is information (Phase 2 sets no speed bar); the numbers
  feed the system check's decision on the owner's side. Each cell: 1 warm-up, then the median of 3 timed
  runs (a single run above RTF 2). The engines run from their own folder with PATH reduced to Windows.

  Get-SpeedPlan and Get-SpeedReportLines are pure and tested. Windows PowerShell 5.1, ASCII only.
#>

Set-StrictMode -Version 2

$script:Inv = [System.Globalization.CultureInfo]::InvariantCulture

function Format-SpeedNum {
  param($Value, [string]$Format = '0.000')
  if ($null -eq $Value) { return 'n/a' }
  ([double]$Value).ToString($Format, $script:Inv)
}

# The models of the table, in order: id, file under models\, display name.
function Get-SpeedModels {
  @(
    [pscustomobject]@{ Id = 'small'; File = 'ggml-small.bin'; Name = 'small' },
    [pscustomobject]@{ Id = 'medium'; File = 'ggml-medium.bin'; Name = 'medium' },
    [pscustomobject]@{ Id = 'large-v3-turbo-q5_0'; File = 'ggml-large-v3-turbo-q5_0.bin'; Name = 'large-v3-turbo q5_0' },
    [pscustomobject]@{ Id = 'large-v3-q5_0'; File = 'ggml-large-v3-q5_0.bin'; Name = 'large-v3 q5_0' },
    [pscustomobject]@{ Id = 'hinglish-q5_0'; File = 'converted\ggml-hinglish-q5_0.bin'; Name = 'hinglish q5_0 (code-switched, large-v3 sized)' }
  )
}

# Pure. The list of cells for one part. -Devices are the Vulkan adapters from `wb-engine-vulkan info`
# (objects with Index, Description, Kind, Driver, Backend). -Present lists the model ids that exist.
function Get-SpeedPlan {
  [CmdletBinding()]
  param(
    [Parameter(Mandatory = $true)] [ValidateSet('igpu', 'cpu')] [string]$Part,
    $Devices = @(),
    [int]$PhysicalCores = 4,
    [int]$LogicalProcessors = 4,
    [bool]$HasAvx512 = $false,
    [bool]$Quick = $false,
    [string[]]$Present = @()
  )
  $models = @(Get-SpeedModels | Where-Object { $Present -contains $_.Id })
  if ($Quick) { $models = @($models | Where-Object { $_.Id -eq 'small' }) }
  $langs = $(if ($Quick) { @('en') } else { @('en', 'hi') })
  $cells = New-Object System.Collections.Generic.List[object]
  $order = 0
  $add = {
    param($row, $variant, $device, $threads, $mode, $m, $lang, $clip)
    $cells.Add([pscustomobject]@{
        Key = ('{0}|{1}|{2}|{3}|{4}|{5}' -f $row, $m.Id, $lang, $clip, $mode, $threads)
        Row = $row; Order = $script:planOrder; Variant = $variant; Device = $device; Threads = $threads; Mode = $mode
        Model = $m; Lang = $lang; Clip = $clip
      })
  }
  $script:planOrder = 0
  if ($Part -eq 'igpu') {
    $gpus = @($Devices | Where-Object { [string]$_.Backend -match 'Vulkan' -and [string]$_.Kind -ne 'cpu' })
    if ($Quick) { $int = @($gpus | Where-Object { [string]$_.Kind -match 'integrated|igpu' }); $gpus = $(if ($int.Count) { @($int[0]) } else { @($gpus | Select-Object -First 1) }) }
    foreach ($g in $gpus) {
      $script:planOrder++
      $row = ('vulkan on {0} (device {1}, driver {2}), 2 CPU threads' -f $g.Description, $g.Index, $(if ($g.Driver) { $g.Driver } else { 'unknown' }))
      foreach ($m in $models) {
        foreach ($l in $langs) { & $add $row 'vulkan' @{ kind = 'gpu'; index = [int]$g.Index } 2 'rolling' $m $l ('bench-{0}.wav' -f $l) }
        if ($m.Id -eq 'hinglish-q5_0' -and -not $Quick) { & $add $row 'vulkan' @{ kind = 'gpu'; index = [int]$g.Index } 2 'rolling' $m 'hi' 'bench-codeswitch.wav' }
      }
    }
  } else {
    # The app's own batch count is one less than the physical cores; the table also shows the physical and logical counts.
    $appThreads = [math]::Max(1, $PhysicalCores - 1)
    $threadSets = @($appThreads, $PhysicalCores, $LogicalProcessors | Where-Object { $_ -ge 1 } | Select-Object -Unique)
    if ($Quick) { $threadSets = @($threadSets | Select-Object -First 1) }
    $variants = @('avx2')
    if ($HasAvx512 -and -not $Quick) { $variants += 'avx512' }
    foreach ($v in $variants) {
      foreach ($th in $threadSets) {
        $script:planOrder++
        $row = ('CPU {0}, {1} threads' -f $v, $th)
        foreach ($m in $models) {
          foreach ($l in $langs) { & $add $row $v @{ kind = 'cpu' } $th 'rolling' $m $l ('bench-{0}.wav' -f $l) }
          if ($m.Id -eq 'hinglish-q5_0' -and -not $Quick) { & $add $row $v @{ kind = 'cpu' } $th 'rolling' $m 'hi' 'bench-codeswitch.wav' }
        }
      }
    }
    if (-not $Quick) {
      $nl = [math]::Max(1, [int][math]::Floor($LogicalProcessors / 2))
      $script:planOrder++
      $small = @($models | Where-Object { $_.Id -eq 'small' })
      foreach ($m in $small) { foreach ($l in $langs) { & $add ('CPU avx2, small near-live (greedy), {0} threads' -f $nl) 'avx2' @{ kind = 'cpu' } $nl 'near_live' $m $l ('bench-{0}.wav' -f $l) } }
    }
  }
  @($cells.ToArray())
}

# Pure. The evidence lines of a part from its result records (objects read from results.jsonl).
function Get-SpeedReportLines {
  [CmdletBinding()]
  param([Parameter(Mandatory = $true)] [string]$Part, $Records = @(), [string]$Machine = '', [string]$Profile = '', [bool]$Quick = $false)
  $rows = @($Records)
  $lines = New-Object System.Collections.Generic.List[string]
  $lines.Add(('Speed table, {0} part (gate pack)' -f $Part))
  $lines.Add(('machine: {0}; profile: {1}{2}' -f $Machine, $Profile, $(if ($Quick) { '; QUICK: one model, one clip (not the full table)' } else { '' })))
  $engines = @($rows | ForEach-Object { '{0} sha256 {1}' -f $_.variant, $_.engine_sha } | Sort-Object -Unique)
  $lines.Add('engines: ' + ($engines -join '; '))
  $wh = @($rows | Where-Object { $_.PSObject.Properties['whisper'] -and $_.whisper } | ForEach-Object { $_.whisper } | Sort-Object -Unique)
  $lines.Add('whisper.cpp printed by the engine: ' + ($wh -join '; '))
  $lines.Add("method: wb-engine bench, the 60 s FLEURS test clips of the app (resources\bench), beam 5 with temperature fallback and 15 s VAD chunks ('rolling') except the near-live rows (greedy); 1 warm-up, then the median of 3 timed runs, a single timed run when the warm-up RTF is above 2; RTF = processing time / clip length, load time reported apart; the engine runs from a folder with only the engine and PATH reduced to the Windows folders; Silero VAD v6.2.0 on the CPU")
  $lines.Add('information only: Phase 2 sets no speed bar. Every row names its adapter and driver version, so the numbers can be compared across laptops and driver updates.')
  $lines.Add('')
  foreach ($g in ($rows | Group-Object row | Sort-Object { [int]$_.Group[0].order })) {
    $lines.Add("== $($g.Name)")
    $lines.Add('model | clip | mode | threads | RTF median (runs) | warm-up RTF | load s | peak working set GiB | peak commit GiB | GPU local / shared GiB | segments | truncated chunks')
    foreach ($r in ($g.Group | Sort-Object model_order, clip)) {
      if ($r.status -eq 'ok') {
        $runs = (@($r.rtf_runs) | ForEach-Object { Format-SpeedNum $_ }) -join ', '
        $gpu = 'n/a'
        if ($null -ne $r.gpu_local_bytes) { $gpu = '{0}/{1}' -f (Format-SpeedNum ($r.gpu_local_bytes / 1GB) '0.00'), (Format-SpeedNum ($r.gpu_shared_bytes / 1GB) '0.00') }
        $single = $(if ($r.single_run) { ' (impractical: single run)' } else { '' })
        $lines.Add(('{0} | {1} | {2} | {3} | {4}{5} ({6}) | {7} | {8} | {9} | {10} | {11} | {12} | {13}' -f $r.model, $r.clip, $r.mode, $r.threads, (Format-SpeedNum $r.rtf_median), $single, $runs, (Format-SpeedNum $r.warmup_rtf), (Format-SpeedNum $r.load_s '0.0'), (Format-SpeedNum ($r.peak_ws / 1GB) '0.00'), (Format-SpeedNum ($r.peak_commit / 1GB) '0.00'), $gpu, $r.segments, $r.truncated))
      } else {
        $word = $(if ($r.status -eq 'does_not_fit') { 'does not fit' } else { "FAILED ($($r.status))" })
        $lines.Add(('{0} | {1} | {2} | {3} | {4} | peak working set (sampled) {5} GiB | peak commit (sampled) {6} GiB | exit code {7} {8}' -f $r.model, $r.clip, $r.mode, $r.threads, $word, (Format-SpeedNum ($r.peak_ws / 1GB) '0.00'), (Format-SpeedNum ($r.peak_commit / 1GB) '0.00'), $r.exit, $r.error))
      }
    }
    $lines.Add('')
  }
  $ok = @($rows | Where-Object { $_.status -eq 'ok' }).Count
  $fit = @($rows | Where-Object { $_.status -eq 'does_not_fit' }).Count
  $lines.Add(('cells: {0} (ok {1}, does not fit {2}, failed {3}, single-run {4})' -f $rows.Count, $ok, $fit, ($rows.Count - $ok - $fit), @($rows | Where-Object { $_.PSObject.Properties['single_run'] -and $_.single_run }).Count))
  $lines.Add('The Hinglish model is large-v3 sized; its code-switched row uses bench-codeswitch.wav (English and Hindi utterances alternating), the others the single-language clips.')
  @($lines.ToArray())
}

# ---- Running the table on this machine (needs the pack's engines and models) ---------------------------

function Invoke-SpeedEngine {
  param([string]$Exe, [string]$ProcName, [string[]]$EngineArgs, [int]$TimeoutMin, [string]$WorkDir, [string]$Emulate = '', [string]$Profile = 'laptop')
  $o = Join-Path $WorkDir 'cell.out'
  $er = Join-Path $WorkDir 'cell.err'
  $file = $Exe
  $argList = $EngineArgs
  if ($Profile -ne 'laptop') { $file = $Emulate; $argList = @('--profile', $Profile, '--', ('"' + $Exe + '"')) + $EngineArgs }
  $saved = $env:PATH
  $env:PATH = "$env:SystemRoot\System32;$env:SystemRoot"
  try { $p = Start-Process -FilePath $file -ArgumentList $argList -PassThru -WindowStyle Hidden -RedirectStandardOutput $o -RedirectStandardError $er }
  finally { $env:PATH = $saved }
  $null = $p.Handle
  $peakWs = 0L
  $peakCommit = 0L
  $watch = [Diagnostics.Stopwatch]::StartNew()
  $timedOut = $false
  while (-not $p.HasExited) {
    foreach ($q in @(Get-Process -Name $ProcName -ErrorAction SilentlyContinue)) {
      try { if ($q.PeakWorkingSet64 -gt $peakWs) { $peakWs = $q.PeakWorkingSet64 }; if ($q.PeakPagedMemorySize64 -gt $peakCommit) { $peakCommit = $q.PeakPagedMemorySize64 } } catch { }
    }
    if ($watch.Elapsed.TotalMinutes -ge $TimeoutMin) {
      $timedOut = $true
      Get-Process -Name $ProcName -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
      try { $p.Kill() } catch { }
      break
    }
    Start-Sleep -Milliseconds 250
  }
  $p.WaitForExit()
  [pscustomobject]@{
    Exit = $(if ($timedOut) { -2 } else { $p.ExitCode }); TimedOut = $timedOut; Seconds = [math]::Round($watch.Elapsed.TotalSeconds, 1)
    Out = @(Get-Content -LiteralPath $o -Encoding UTF8 -ErrorAction SilentlyContinue); Err = @(Get-Content -LiteralPath $er -Encoding UTF8 -ErrorAction SilentlyContinue)
    PeakWs = $peakWs; PeakCommit = $peakCommit
  }
}

function Read-SpeedEvents {
  param($Lines)
  @($Lines | Where-Object { $_ -match '^\{' } | ForEach-Object { try { $_ | ConvertFrom-Json } catch { $null } } | Where-Object { $_ })
}

# Runs `info` of an engine variant and returns its devices (Backend, Description, Kind, Driver, Index).
function Get-SpeedDevices {
  param([string]$Exe, [string]$ProcName, [string]$WorkDir, [string]$Emulate = '', [string]$Profile = 'laptop')
  $r = Invoke-SpeedEngine -Exe $Exe -ProcName $ProcName -EngineArgs @('info') -TimeoutMin 5 -WorkDir $WorkDir -Emulate $Emulate -Profile $Profile
  $info = Read-SpeedEvents $r.Out | Where-Object { $_.event -eq 'info' } | Select-Object -First 1
  if (-not $info) { return @() }
  @($info.devices | ForEach-Object { [pscustomobject]@{ Index = [int]$_.index; Backend = [string]$_.backend; Description = [string]$_.description; Kind = [string]$_.kind; Driver = [string]$_.driver_version } })
}

# The body of the speed-table-igpu and speed-table-cpu gates. Returns a pack result (INFO when cells
# ran, INVALID when none did). Cells already in results.jsonl are skipped, so a repeated gate resumes.
function Invoke-SpeedTableGate {
  [CmdletBinding()]
  param([Parameter(Mandatory = $true)] $Ctx, [Parameter(Mandatory = $true)] [ValidateSet('igpu', 'cpu')] [string]$Part)
  Import-Module (Join-Path $PSScriptRoot 'PackCore.psm1')
  Import-Module (Join-Path $PSScriptRoot 'PackGate.psm1')
  Import-Module (Join-Path $PSScriptRoot 'GateRun.psm1')
  Import-Module (Join-Path $PSScriptRoot 'Redact.psm1')
  $id = $(if ($Part -eq 'igpu') { 'speed-table-igpu' } else { 'speed-table-cpu' })
  $paths = Get-PackPaths -Ctx $Ctx
  $quick = [bool]$Ctx.quick
  $gdir = [string]$Ctx.gate_dir
  $work = Join-Path $gdir 'work'
  New-Item -ItemType Directory -Force -Path $work | Out-Null
  $profile = [string]$Ctx.profile
  $triple = 'x86_64-pc-windows-msvc'
  $fp = Read-PackJson -Path (Join-Path ([string]$Ctx.work_dir) 'fingerprint.json')
  if ($null -eq $fp) { throw 'engines-info has not run on this laptop yet (no fingerprint.json); run it first' }
  if ($profile -ne 'laptop' -and (-not $paths.Emulate -or -not (Test-Path -LiteralPath $paths.Emulate))) { throw "the emulation profile $profile needs wb-emulate; it is not available" }
  $vad = Join-Path $paths.Models 'ggml-silero-v6.2.0.bin'
  if (-not (Test-Path -LiteralPath $vad)) { throw "the VAD model is missing from the pack: $vad" }
  $present = @(Get-SpeedModels | Where-Object { Test-Path -LiteralPath (Join-Path $paths.Models $_.File) } | ForEach-Object { $_.Id })
  $missing = @(Get-SpeedModels | Where-Object { $present -notcontains $_.Id } | ForEach-Object { $_.Id })

  # Engines: one folder and one unique exe name per variant (the exe is found by name for the memory samples).
  $engines = @{}
  foreach ($v in @('avx2', 'avx512', 'vulkan')) {
    $src = Join-Path $paths.Engines "wb-engine-$v-$triple.exe"
    if (-not (Test-Path -LiteralPath $src)) { continue }
    $dir = Join-Path $work "engine-$v"
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    $exe = Join-Path $dir "wb-engine-speed-$v.exe"
    Copy-Item -LiteralPath $src -Destination $exe -Force
    $engines[$v] = @{ Exe = $exe; Name = "wb-engine-speed-$v"; Sha = (Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash.ToLowerInvariant() }
  }

  $devices = @()
  if ($Part -eq 'igpu') {
    if (-not $engines.ContainsKey('vulkan')) { throw 'the vulkan engine is not in this pack' }
    $devices = @(Get-SpeedDevices -Exe $engines['vulkan'].Exe -ProcName $engines['vulkan'].Name -WorkDir $work -Emulate $paths.Emulate -Profile $profile)
    if (@($devices | Where-Object { $_.Backend -match 'Vulkan' -and $_.Kind -ne 'cpu' }).Count -eq 0) {
      return (New-PackResult -Id $id -Status 'INVALID' -Headline 'the vulkan engine lists no graphics adapter on this laptop (check the graphics driver)' -Quick $quick)
    }
  }
  $physCores = [int]$fp.physical_cores
  $logProcs = [int]$fp.logical_processors
  if ($profile -ne 'laptop') {
    # Under wb-emulate the engine sees only the emulated processors. Thread counts from the host
    # fingerprint (24 on the Legion) would spin 24 threads on 4 processors and never finish.
    $probe = $(if ($engines.ContainsKey('avx2')) { $engines['avx2'] } else { $engines.Values | Select-Object -First 1 })
    if ($probe) {
      $r = Invoke-SpeedEngine -Exe $probe.Exe -ProcName $probe.Name -EngineArgs @('info') -TimeoutMin 5 -WorkDir $work -Emulate $paths.Emulate -Profile $profile
      $pi = Read-SpeedEvents $r.Out | Where-Object { $_.event -eq 'info' } | Select-Object -First 1
      if ($pi -and [int]$pi.logical_processors_allowed -ge 1) {
        $logProcs = [int]$pi.logical_processors_allowed
        $physCores = [math]::Min($physCores, $logProcs)
      }
    }
  }
  $plan = @(Get-SpeedPlan -Part $Part -Devices $devices -PhysicalCores $physCores -LogicalProcessors $logProcs -HasAvx512 $(if ($fp.PSObject.Properties['avx512_usable']) { [bool]$fp.avx512_usable } else { [bool]$fp.avx512f }) -Quick $quick -Present $present)
  $plan = @($plan | Where-Object { $engines.ContainsKey($_.Variant) })
  if ($plan.Count -eq 0) { throw 'no cell can run: the engines or models of this part are not in the pack' }
  $resFile = Join-Path $gdir 'results.jsonl'
  $inv = [System.Globalization.CultureInfo]::InvariantCulture
  $clipSec = @{}
  foreach ($c in ($plan | Select-Object -ExpandProperty Clip -Unique)) {
    $f = Join-Path $paths.BenchClips $c
    if (-not (Test-Path -LiteralPath $f)) { throw "the test clip $c is missing from the pack" }
    $clipSec[$c] = [math]::Round(((Get-Item -LiteralPath $f).Length - 44) / 2 / 16000, 1)
  }
  Write-PackGateLog -Ctx $Ctx -Message ("{0}: {1} cells planned{2}" -f $id, $plan.Count, $(if ($missing.Count) { "; models not in the pack: $($missing -join ', ')" } else { '' }))

  Invoke-WbGate -Name $id -Body {
    $n = 0
    foreach ($cell in $plan) {
      $n++
      if (Test-Path -LiteralPath $resFile) {
        $done = @(Get-Content -LiteralPath $resFile -Encoding UTF8 | Where-Object { $_.Trim() } | ForEach-Object { $_ | ConvertFrom-Json } | Where-Object { $_.key -eq $cell.Key -and $_.status -in @('ok', 'does_not_fit', 'timeout') })
        if ($done.Count) { continue }
      }
      $req = [ordered]@{
        type = 'bench'
        model = @{ id = $cell.Model.Id; path = (Join-Path $paths.Models $cell.Model.File) }
        vad_model = @{ id = 'silero-v6.2.0'; path = $vad }
        lang = $cell.Lang; clip = (Join-Path $paths.BenchClips $cell.Clip); mode = $cell.Mode; device = $cell.Device; threads = $cell.Threads; runs = $(if ($quick) { 1 } else { 3 })
        single_run_above_rtf = 2.0
      }
      $reqFile = Join-Path $work 'request.json'
      [IO.File]::WriteAllText($reqFile, ($req | ConvertTo-Json -Depth 6), (New-Object Text.UTF8Encoding($false)))
      Write-PackGateLog -Ctx $Ctx -Message ("{0}/{1} {2}" -f $n, $plan.Count, $cell.Key)
      $e = $engines[$cell.Variant]
      $r = Invoke-SpeedEngine -Exe $e.Exe -ProcName $e.Name -EngineArgs @('bench', '--request', ('"' + $reqFile + '"')) -TimeoutMin 60 -WorkDir $work -Emulate $paths.Emulate -Profile $profile
      $ev = Read-SpeedEvents $r.Out
      $b = $ev | Where-Object { $_.event -eq 'bench' } | Select-Object -First 1
      $errEv = $ev | Where-Object { $_.event -eq 'error' } | Select-Object -First 1
      $modelOrder = @(Get-SpeedModels | ForEach-Object { $_.Id }).IndexOf($cell.Model.Id)
      $rec = [ordered]@{
        key = $cell.Key; row = $cell.Row; order = $cell.Order; model = $cell.Model.Name; model_order = $modelOrder; lang = $cell.Lang; clip = $cell.Clip; mode = $cell.Mode; threads = $cell.Threads
        variant = $cell.Variant; engine_sha = $e.Sha; profile = $profile; when = (Get-Date).ToString('yyyy-MM-dd HH:mm', $inv)
        clip_seconds = $clipSec[$cell.Clip]; exit = $r.Exit; seconds = $r.Seconds; peak_ws = $r.PeakWs; peak_commit = $r.PeakCommit
      }
      if ($b -and $r.Exit -eq 0) {
        $rec.status = 'ok'; $rec.rtf_median = $b.rtf_median; $rec.rtf_runs = @($b.rtf_runs); $rec.warmup_rtf = $b.warmup_rtf; $rec.single_run = [bool]$b.single_run
        $rec.load_s = $b.load_s; $rec.peak_ws = [int64]$b.peak_working_set_bytes; $rec.peak_commit = [int64]$b.peak_commit_bytes
        $rec.gpu_local_bytes = $b.gpu_local_bytes; $rec.gpu_shared_bytes = $b.gpu_shared_bytes; $rec.segments = $b.segments; $rec.truncated = $b.truncated_chunks
        $rec.whisper = "$($b.whisper_cpp.version) commit $($b.whisper_cpp.commit) tree $($b.whisper_cpp.tree_sha256)"; $rec.device_name = $b.device_name
      } else {
        $msg = $(if ($errEv) { "$($errEv.code): $($errEv.message)" } else { ($r.Err | Select-Object -Last 2) -join ' ' })
        $rec.status = $(if ($r.TimedOut) { 'timeout' } elseif ($msg -match 'out of memory|bad_alloc|not enough memory|failed to allocate|VK_ERROR_OUT_OF|OutOfDeviceMemory|OutOfHostMemory') { 'does_not_fit' } else { "exit_$($r.Exit)" })
        $rec.error = $msg -replace '[\r\n]+', ' '
      }
      ($rec | ConvertTo-Json -Compress -Depth 5) | Add-Content -LiteralPath $resFile -Encoding ASCII
    }
  } | Out-Null

  $all = @()
  if (Test-Path -LiteralPath $resFile) { $all = @(Get-Content -LiteralPath $resFile -Encoding UTF8 | Where-Object { $_.Trim() } | ForEach-Object { $_ | ConvertFrom-Json }) }
  $recs = @($all | Group-Object key | ForEach-Object { $_.Group | Select-Object -Last 1 })
  $lines = @(Get-SpeedReportLines -Part $Part -Records $recs -Machine ([string]$Ctx.machine) -Profile $profile -Quick $quick)
  if ($Part -eq 'igpu') {
    $lines += ''
    $lines += 'graphics adapters seen by wb-engine-vulkan info:'
    foreach ($d in $devices) { $lines += ('  device {0}: {1} / {2} / {3} / driver {4}' -f $d.Index, $d.Backend, $d.Description, $d.Kind, $d.Driver) }
  }
  if ($missing.Count) { $lines += ''; $lines += ('models not in this pack, so not measured: ' + ($missing -join ', ')) }
  $evidence = Join-Path $gdir ($id + '.txt')
  Write-WbEvidence -Path $evidence -Lines $lines -MachineLabel ([string]$Ctx.machine)
  $ok = @($recs | Where-Object { $_.status -eq 'ok' }).Count
  if ($ok -eq 0) {
    return (New-PackResult -Id $id -Status 'INVALID' -Headline ('no cell produced a result ({0} cells tried); see {1}' -f $recs.Count, $evidence) -Evidence @($evidence, $resFile) -Quick $quick)
  }
  $worst = ($recs | Where-Object { $_.status -eq 'ok' } | Measure-Object -Property rtf_median -Maximum).Maximum
  $notes = @()
  if ($missing.Count) { $notes = @("models not in this pack: $($missing -join ', ')") }
  $head = ('{0} of {1} cells measured; slowest RTF {2}; information only' -f $ok, $plan.Count, (Format-SpeedNum $worst))
  New-PackResult -Id $id -Status 'INFO' -Headline $head -Measured ([ordered]@{ cells_planned = $plan.Count; cells_ok = $ok; slowest_rtf = $worst; adapters = @($devices | ForEach-Object { [ordered]@{ description = $_.Description; kind = $_.Kind; driver = $_.Driver } }) }) -Threshold 'information: RTF, load time, memory and adapter driver per cell; no verdict' -Evidence @($evidence, $resFile) -Notes $notes -Quick $quick
}

Export-ModuleMember -Function Get-SpeedModels, Get-SpeedPlan, Get-SpeedReportLines, Invoke-SpeedEngine, Read-SpeedEvents, Get-SpeedDevices, Format-SpeedNum, Invoke-SpeedTableGate
