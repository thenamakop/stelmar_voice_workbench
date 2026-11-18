<#
  SpeedBilingual.psm1: the pure parts of the speed-bilingual gate (information only, no bar).

  Question: how much does per-chunk language detection slow real queue work on THIS laptop? The system check
  benches one language with no detection; the queue of a two-language session detects the language of every
  chunk. The measurement is on the SAME audio, so the clip's content cancels out: bench-codeswitch.wav, looped
  into a longer input in the gate's work folder (never in the pack), is decoded twice on the engine's own
  `batch` command with the request the queue builds (crates/wb-runner/src/exec.rs build_request: Settings::default
  with the role, sentence-sized chunks, VAD chunking):
    fixed)  the language fixed to hi, no detection (the engine refuses several languages with detection off);
    detect) languages en+hi, detection on every chunk, as the queue does for a two-language session.
  Done for rolling (the quick check's models; 5 min full, 2 min quick) and for night (turbo for English and
  large-v3 for Hindi, as the queue's night engine loads them; 2 min full, 1 min quick). The factor and the
  seconds per detection come from fixed against detect only. bench-hi.wav (rolling, fixed hi) is a reference line.
  The device, backend build, thread count and the rolling model of each language are the cell the laptop's own
  quick check chose (capability.json). Nothing is hard-coded for a class of machine.

  Windows PowerShell 5.1, ASCII only.
#>
$ErrorActionPreference = 'Stop'
$script:Inv = [Globalization.CultureInfo]::InvariantCulture
$script:NightModels = [ordered]@{ en = 'large-v3-turbo-q5_0'; hi = 'large-v3-q5_0' }

function Get-BilingualNightModels { $script:NightModels }

# The decoding settings of the queue for a role: Settings::default of wb-protocol with the role set (the app's
# executor sets nothing else but the glossary, which these clips do not have).
function Get-BilingualSettings {
  param([ValidateSet('rolling', 'night')] [string]$Role)
  [ordered]@{
    role = $Role; beam_size = 5; best_of = 5; temperature_fallback = $true; previous_text_context = $true; context_chars = 200
    timestamps = $false; glossary = ''; max_chunk_s = 15.0; sentence_split = $true; min_chunk_s = 2.0; vad_threshold = 0.5
    vad_min_silence_ms = 100; flash_attn = $true
  }
}

# The rolling cell this laptop's quick check chose for one language: the measurement of the decision's rolling
# model (matched on the decision's own RTF, else the fastest rolling measurement of that model and language).
# Returns $null when the check chose no rolling model for the language.
function Get-BilingualCell {
  param($Capability, [string]$Lang)
  if ($null -eq $Capability) { return $null }
  $dec = $Capability.decision
  if ($null -eq $dec -or $null -eq $dec.languages) { return $null }
  $l = $dec.languages.PSObject.Properties[$Lang]
  if ($null -eq $l -or $null -eq $l.Value -or $null -eq $l.Value.rolling) { return $null }
  $pick = $l.Value.rolling
  $model = [string]$pick.model
  $cands = @($Capability.measurements | Where-Object { $_.model -eq $model -and $_.lang -eq $Lang -and $_.mode -eq 'rolling' -and -not $_.live_standin })
  if ($cands.Count -eq 0) { return $null }
  $m = $null
  if ($null -ne $pick.rtf) {
    $hit = @($cands | Where-Object { [math]::Abs([double]$_.rtf - [double]$pick.rtf) -lt 1e-6 } | Select-Object -First 1)
    if ($hit.Count -gt 0) { $m = $hit[0] }
  }
  if ($null -eq $m) { $m = @($cands | Sort-Object { [double]$_.rtf } | Select-Object -First 1)[0] }
  $kind = 'cpu'
  $index = $null
  if ($m.config -and $m.config.kind) { $kind = [string]$m.config.kind }
  if ($m.config -and $null -ne $m.config.PSObject.Properties['index']) { $index = [int]$m.config.index }
  [pscustomobject]@{
    Lang = $Lang; Model = $model; Rtf = [double]$m.rtf; Variant = [string]$m.engine_build; DeviceKind = $kind; GpuIndex = $index
    Threads = [int]$m.threads; DeviceName = [string]$m.device_name
  }
}

# The device object of a request.
function Get-BilingualDevice {
  param($Cell)
  if ($Cell.DeviceKind -eq 'gpu' -and $null -ne $Cell.GpuIndex) { return [ordered]@{ kind = 'gpu'; index = [int]$Cell.GpuIndex } }
  [ordered]@{ kind = 'cpu' }
}

# Pure. How long the looped input of each part is, in seconds (owner, 18 Nov 2026): rolling 5 min full / 2 min quick,
# night 2 min full / 1 min quick. bench-codeswitch has 4 sentence-sized chunks a minute, so full mode gives about 20
# detections for rolling and about 8 for night; the report says how many there were.
function Get-BilingualLoopSeconds {
  param([ValidateSet('rolling', 'night')] [string]$Part, [bool]$Quick)
  if ($Part -eq 'rolling') { if ($Quick) { return 120 } else { return 300 } }
  if ($Quick) { return 60 } else { return 120 }
}

# Pure. The measurements to make. -Hi and -En are cells from Get-BilingualCell. All run on the Hindi cell's engine,
# device and threads (one engine holds both models, as the queue starts it). The measurements of one part use the
# SAME audio: the looped bench-codeswitch.wav (LoopSeconds), once with the language fixed to hi and no detection,
# once with en+hi and detection on every chunk. The reference line is bench-hi.wav, rolling, fixed hi, one clip,
# only when it costs under 45 s (the cell's own RTF times the clip).
# Returns objects: Key, Part (rolling|night|reference), Mode (fixed|detect), Clip, LoopSeconds ($null: the clip as it is), Role,
# Languages, Models (lang -> id), Detect, Repeats, Label.
function Get-BilingualPlan {
  param($Hi, $En, [bool]$Quick = $false, [double]$ClipSeconds = 60)
  $reps = $(if ($Quick) { 1 } else { 2 })
  $plan = New-Object System.Collections.Generic.List[object]
  if ($null -eq $Hi) { return @() }
  $nm = Get-BilingualNightModels
  $rl = Get-BilingualLoopSeconds -Part 'rolling' -Quick $Quick
  $nl = Get-BilingualLoopSeconds -Part 'night' -Quick $Quick
  $plan.Add([pscustomobject]@{ Key = 'rolling-fixed-hi'; Part = 'rolling'; Mode = 'fixed'; Clip = 'bench-codeswitch.wav'; LoopSeconds = $rl; Role = 'rolling'; Languages = @('hi'); Models = [ordered]@{ hi = $Hi.Model }; Detect = $false; Repeats = $reps
      Label = 'rolling, bench-codeswitch looped, language fixed to hi, no detection' })
  if ($En) {
    $plan.Add([pscustomobject]@{ Key = 'rolling-detect-en-hi'; Part = 'rolling'; Mode = 'detect'; Clip = 'bench-codeswitch.wav'; LoopSeconds = $rl; Role = 'rolling'; Languages = @('en', 'hi'); Models = [ordered]@{ en = $En.Model; hi = $Hi.Model }; Detect = $true; Repeats = $reps
        Label = 'rolling, bench-codeswitch looped, languages en+hi, detection on every chunk' })
  }
  $plan.Add([pscustomobject]@{ Key = 'night-fixed-hi'; Part = 'night'; Mode = 'fixed'; Clip = 'bench-codeswitch.wav'; LoopSeconds = $nl; Role = 'night'; Languages = @('hi'); Models = [ordered]@{ hi = $nm['hi'] }; Detect = $false; Repeats = $reps
      Label = 'night, bench-codeswitch looped, language fixed to hi, no detection' })
  $plan.Add([pscustomobject]@{ Key = 'night-detect-en-hi'; Part = 'night'; Mode = 'detect'; Clip = 'bench-codeswitch.wav'; LoopSeconds = $nl; Role = 'night'; Languages = @('en', 'hi'); Models = [ordered]@{ en = $nm['en']; hi = $nm['hi'] }; Detect = $true; Repeats = $reps
      Label = 'night, bench-codeswitch looped, languages en+hi (turbo for en, large-v3 for hi), detection on every chunk' })
  if (($Hi.Rtf * $ClipSeconds) -le 45) {
    $plan.Add([pscustomobject]@{ Key = 'reference-bench-hi'; Part = 'reference'; Mode = 'fixed'; Clip = 'bench-hi.wav'; LoopSeconds = $null; Role = 'rolling'; Languages = @('hi'); Models = [ordered]@{ hi = $Hi.Model }; Detect = $false; Repeats = $reps
        Label = 'reference: rolling, bench-hi (one Hindi clip), language fixed to hi, no detection' })
  }
  @($plan.ToArray())
}

# The batch request the queue would send for one measurement (wb-protocol BatchRequest, tagged type batch).
function New-BilingualRequest {
  param($Measurement, $Cell, [string]$ClipPath, [hashtable]$ModelPaths, [string]$VadPath)
  $mbl = [ordered]@{}
  foreach ($k in $Measurement.Models.Keys) { $id = [string]$Measurement.Models[$k]; $mbl[$k] = [ordered]@{ id = $id; path = [string]$ModelPaths[$id] } }
  [ordered]@{
    type = 'batch'
    source = [ordered]@{ kind = 'file'; path = $ClipPath }
    units = @()
    models_by_lang = $mbl
    vad_model = [ordered]@{ id = 'silero-v6.2.0'; path = $VadPath }
    languages = @($Measurement.Languages)
    lang_log = @()
    settings = (Get-BilingualSettings -Role $Measurement.Role)
    device = (Get-BilingualDevice -Cell $Cell)
    threads = [int]$Cell.Threads
    resume_from_ms = 0
    detect_language = [bool]$Measurement.Detect
  }
}

# Pure. What one engine run says, from its events (hello, ready, segment, stats, done, error).
function Get-BilingualRunStats {
  param($Events)
  $st = @($Events | Where-Object { $_.event -eq 'stats' } | Select-Object -First 1)
  $ready = @($Events | Where-Object { $_.event -eq 'ready' } | Select-Object -First 1)
  $done = @($Events | Where-Object { $_.event -eq 'done' } | Select-Object -First 1)
  $err = @($Events | Where-Object { $_.event -eq 'error' } | Select-Object -First 1)
  $hello = @($Events | Where-Object { $_.event -eq 'hello' } | Select-Object -First 1)
  $segs = @($Events | Where-Object { $_.event -eq 'segment' })
  $r = [ordered]@{ ok = $false; error = ''; rtf = $null; decode_s = $null; audio_ms = $null; chunks = $null; detections = $null; both_ways = $null; load_s = $null; device_name = ''; segments = $segs.Count; peak_working_set_bytes = $null; whisper = '' }
  if ($err.Count -gt 0 -and $err[0]) { $r.error = ('{0}: {1}' -f $err[0].code, $err[0].message) }
  if ($ready.Count -gt 0 -and $ready[0]) { $r.load_s = [double]$ready[0].load_s; $r.device_name = [string]$ready[0].device_name }
  if ($hello.Count -gt 0 -and $hello[0] -and $hello[0].PSObject.Properties['whisper_cpp'] -and $hello[0].whisper_cpp) { $w = $hello[0].whisper_cpp; $r.whisper = ('{0} commit {1}' -f $w.version, $w.commit) }
  if ($st.Count -gt 0 -and $st[0]) {
    $r.rtf = [double]$st[0].rtf; $r.decode_s = [double]$st[0].decode_s; $r.audio_ms = [int64]$st[0].audio_ms; $r.chunks = [int]$st[0].chunks
    $r.detections = [int]$st[0].detections; $r.both_ways = [int]$st[0].both_ways; $r.peak_working_set_bytes = [int64]$st[0].peak_working_set_bytes
  }
  $r.ok = ($st.Count -gt 0 -and $done.Count -gt 0 -and $err.Count -eq 0 -and $null -ne $r.rtf -and $r.audio_ms -gt 0)
  [pscustomobject]$r
}

function Get-BilingualMedian {
  param([double[]]$Values)
  $v = @($Values | Sort-Object)
  if ($v.Count -eq 0) { return $null }
  if ($v.Count % 2 -eq 1) { return $v[[int](($v.Count - 1) / 2)] }
  ($v[$v.Count / 2 - 1] + $v[$v.Count / 2]) / 2
}

# Pure. One part (rolling or night) from the runs of its fixed and its detect measurement. Both decoded the SAME audio, so
# the factor and the time per detection come from these two only. -FixedRuns and -DetectRuns are arrays of
# Get-BilingualRunStats objects (plain arrays: @() around a generic List fails with "Argument types do not match").
#   Factor            detect RTF over fixed RTF (medians)
#   DeltaS            engine seconds with detection minus engine seconds fixed (medians), the same audio
#   NoiseS            what the two measurements cannot tell apart: the larger spread between a measurement's own runs
#                     (full mode has two), and never less than 3 percent of the fixed time
#   WithinNoise       DeltaS is not larger than NoiseS: the result line then says "no extra time seen"
#   ExtraPerDetection DeltaS / detections, kept (also when negative) in the evidence
function Get-BilingualPart {
  param($FixedRuns, $DetectRuns)
  $f = @($FixedRuns | Where-Object { $_ -and $_.ok })
  $d = @($DetectRuns | Where-Object { $_ -and $_.ok })
  if ($f.Count -eq 0 -or $d.Count -eq 0) { return [pscustomobject]@{ Ok = $false; FixedRtf = $null; DetectRtf = $null; Factor = $null; Detections = $null; DetectionsPerMin = $null; AudioS = $null; FixedS = $null; DetectS = $null; DeltaS = $null; NoiseS = $null; WithinNoise = $null; ExtraPerDetection = $null; FixedRuns = $f.Count; DetectRuns = $d.Count } }
  $fr = Get-BilingualMedian -Values @($f | ForEach-Object { [double]$_.rtf })
  $dr = Get-BilingualMedian -Values @($d | ForEach-Object { [double]$_.rtf })
  $fs = Get-BilingualMedian -Values @($f | ForEach-Object { [double]$_.decode_s })
  $ds = Get-BilingualMedian -Values @($d | ForEach-Object { [double]$_.decode_s })
  $det = [int][math]::Round((Get-BilingualMedian -Values @($d | ForEach-Object { [double]$_.detections })))
  $audioS = (Get-BilingualMedian -Values @($d | ForEach-Object { [double]$_.audio_ms })) / 1000.0
  $factor = $null
  if ($fr -gt 0) { $factor = $dr / $fr }
  $perMin = $null
  if ($audioS -gt 0) { $perMin = $det / ($audioS / 60.0) }
  $delta = $ds - $fs
  $fv = @($f | ForEach-Object { [double]$_.decode_s })
  $dv = @($d | ForEach-Object { [double]$_.decode_s })
  $spreadF = ($fv | Measure-Object -Maximum).Maximum - ($fv | Measure-Object -Minimum).Minimum
  $spreadD = ($dv | Measure-Object -Maximum).Maximum - ($dv | Measure-Object -Minimum).Minimum
  $noise = [math]::Max([math]::Max($spreadF, $spreadD), 0.03 * $fs)
  $extra = $null
  if ($det -gt 0) { $extra = $delta / $det }
  [pscustomobject]@{ Ok = $true; FixedRtf = $fr; DetectRtf = $dr; Factor = $factor; Detections = $det; DetectionsPerMin = $perMin; AudioS = $audioS; FixedS = $fs; DetectS = $ds; DeltaS = $delta; NoiseS = $noise; WithinNoise = ($delta -le $noise); ExtraPerDetection = $extra; FixedRuns = $f.Count; DetectRuns = $d.Count }
}

function Format-BilingualNum {
  param($Value, [string]$Format = '0.00')
  if ($null -eq $Value) { return 'n/a' }
  ([double]$Value).ToString($Format, $script:Inv)
}

# Pure. "rolling RTF 0.80 fixed-hi / 0.95 en+hi with detection (x1.19; 20 detections, 0.8 s each)". A time per detection
# that is negative or within noise is not printed: "no extra time seen" (the evidence keeps the number).
function Format-BilingualPartText {
  param($Part, [string]$Name)
  if ($null -eq $Part -or -not $Part.Ok) { return ('{0} not measured' -f $Name) }
  $each = ''
  if ($Part.Detections -gt 0) {
    if ($Part.WithinNoise -or $null -eq $Part.ExtraPerDetection -or $Part.ExtraPerDetection -le 0) { $each = ', no extra time seen' }
    else { $each = ', {0} s each' -f (Format-BilingualNum $Part.ExtraPerDetection '0.0') }
  }
  '{0} RTF {1} fixed-hi / {2} en+hi with detection (x{3}; {4} detections{5})' -f $Name, (Format-BilingualNum $Part.FixedRtf), (Format-BilingualNum $Part.DetectRtf), (Format-BilingualNum $Part.Factor), $Part.Detections, $each
}

# Pure. The result line: "rolling RTF 0.80 fixed-hi / 0.95 en+hi with detection (x1.19; 20 detections, 0.8 s each);
# night RTF 1.54 fixed-hi / 1.70 en+hi with detection (x1.10; 8 detections, 1.5 s each); vulkan gpu 0, hi medium / en small, 2 threads".
function Format-BilingualHeadline {
  param($Rolling, $Night, [string]$Where, [string[]]$NotMeasured = @())
  $parts = New-Object System.Collections.Generic.List[string]
  $parts.Add((Format-BilingualPartText -Part $Rolling -Name 'rolling'))
  $parts.Add((Format-BilingualPartText -Part $Night -Name 'night'))
  if ($Where) { $parts.Add($Where) }
  $s = ($parts -join '; ')
  if ($NotMeasured.Count -gt 0) { $s += ' [' + ($NotMeasured -join '; ') + ']' }
  $s
}

# Pure. "vulkan gpu 0, hi medium / en small, 2 threads" (the cell this laptop's quick check chose).
function Format-BilingualWhere {
  param($Hi, $En)
  $dev = 'cpu'
  if ($Hi.DeviceKind -eq 'gpu') { $dev = 'gpu ' + $Hi.GpuIndex }
  $enText = ''
  if ($En) { $enText = ' / en ' + $En.Model }
  '{0} {1}, hi {2}{3}, {4} threads' -f $Hi.Variant, $dev, $Hi.Model, $enText, $Hi.Threads
}

# Pure. The evidence lines (no transcript text; numbers and the request's shape only).
# -Records: one per measurement with key, label, models, clip, input, languages, detect, runs (Get-BilingualRunStats objects).
function Get-BilingualReportLines {
  param($Records, $Rolling, $Night, [string]$Machine, [string]$Profile, [bool]$Quick, $Hi, $En, [string[]]$Notes = @())
  $L = New-Object System.Collections.Generic.List[string]
  $mode = 'full (each measurement twice, median reported)'
  if ($Quick) { $mode = 'quick (each measurement once)' }
  $L.Add('Speed with and without per-chunk language detection, on the same audio (gate pack, information only, no bar)')
  $L.Add(('machine: {0}; profile: {1}; mode: {2}; date: {3}' -f $Machine, $Profile, $mode, (Get-Date).ToString('yyyy-MM-dd HH:mm', $script:Inv)))
  if ($Hi) { $L.Add(('cell chosen by this laptop''s quick check: {0}' -f (Format-BilingualWhere -Hi $Hi -En $En))) }
  $L.Add('request: the queue''s (Settings::default with the role, beam 5, best of 5, sentence-sized chunks); detection on only where two languages are given')
  $L.Add('RTF here = engine seconds for the unit (VAD, detection, decoding; not model load) / audio seconds')
  $L.Add('')
  foreach ($r in $Records) {
    $L.Add(('{0}: {1}' -f $r.key, $r.label))
    $det = 'off'
    if ($r.detect) { $det = 'on' }
    $L.Add(('   models: {0}; input: {1}; languages: {2}; detection: {3}' -f $r.models, $r.input, $r.languages, $det))
    $i = 0
    foreach ($x in @($r.runs)) {
      $i++
      if ($x.ok) { $L.Add(('   run {0}: RTF {1} ({2} detections, {3} chunks, {4} s)' -f $i, (Format-BilingualNum $x.rtf '0.000'), $x.detections, $x.chunks, (Format-BilingualNum $x.decode_s '0.0'))) }
      else { $L.Add(('   run {0}: failed: {1}' -f $i, $x.error)) }
    }
    $okRuns = @($r.runs | Where-Object { $_.ok })
    if ($okRuns.Count -gt 1) { $L.Add(('   median RTF: {0}' -f (Format-BilingualNum (Get-BilingualMedian -Values @($okRuns | ForEach-Object { [double]$_.rtf })) '0.000'))) }
  }
  $L.Add('')
  foreach ($p in @(@('rolling', $Rolling), @('night', $Night))) {
    $x = $p[1]
    if ($x -and $x.Ok) {
      $L.Add(('{0}: fixed RTF {1} ({2} s); with detection RTF {3} ({4} s) for {5} s of audio; factor x{6}' -f $p[0], (Format-BilingualNum $x.FixedRtf '0.000'), (Format-BilingualNum $x.FixedS '0.0'), (Format-BilingualNum $x.DetectRtf '0.000'), (Format-BilingualNum $x.DetectS '0.0'), (Format-BilingualNum $x.AudioS '0'), (Format-BilingualNum $x.Factor)))
      $verdict = 'outside the noise: detection added time'
      if ($x.WithinNoise) { $verdict = 'within noise: no extra time seen' }
      $L.Add(('{0}: {1} detections ({2} per minute of audio); time with detection minus time fixed = {3} s; {4} s per detection; noise {5} s ({6})' -f $p[0], $x.Detections, (Format-BilingualNum $x.DetectionsPerMin '0.0'), (Format-BilingualNum $x.DeltaS '0.0'), (Format-BilingualNum $x.ExtraPerDetection '0.00'), (Format-BilingualNum $x.NoiseS '0.0'), $verdict))
    } else { $L.Add(('{0}: not measured (no complete pair of runs)' -f $p[0])) }
  }
  $ref = @($Records | Where-Object { $_.key -eq 'reference-bench-hi' } | Select-Object -First 1)
  if ($ref.Count -gt 0 -and $ref[0]) {
    $ok = @($ref[0].runs | Where-Object { $_.ok })
    if ($ok.Count -gt 0) { $L.Add(('reference (bench-hi.wav, one Hindi clip, rolling, language fixed to hi): RTF {0}; not used in any factor' -f (Format-BilingualNum (Get-BilingualMedian -Values @($ok | ForEach-Object { [double]$_.rtf })) '0.000'))) }
  }
  $L.Add('seconds per detection = (engine seconds with detection - engine seconds fixed) / detections, both on the same looped audio; a time that is negative or inside the noise is reported as no extra time seen.')
  $L.Add('the fixed run forces Hindi on the English parts of the clip (the engine refuses several languages with detection off), so it is the closest same-audio baseline, not a transcript-quality comparison.')
  foreach ($n in $Notes) { $L.Add('note: ' + $n) }
  $L.Add('information only: no verdict')
  @($L.ToArray())
}

# ---- Looping a WAV (the input of the measurements is built in the gate's work folder, never in the pack) ----

# Reads a PCM WAV: the fmt chunk bytes, the data bytes, the byte rate and the length in seconds.
function Read-BilingualWav {
  param([Parameter(Mandatory = $true)] [string]$Path)
  $b = [IO.File]::ReadAllBytes($Path)
  $asc = [Text.Encoding]::ASCII
  if ($b.Length -lt 44 -or $asc.GetString($b, 0, 4) -ne 'RIFF' -or $asc.GetString($b, 8, 4) -ne 'WAVE') { throw "$Path is not a WAV file" }
  $pos = 12L
  $fmt = $null
  $data = $null
  while (($pos + 8) -le $b.Length) {
    $id = $asc.GetString($b, [int]$pos, 4)
    $len = [int64][BitConverter]::ToUInt32($b, [int]($pos + 4))
    $body = $pos + 8
    if ($id -eq 'fmt ') { $fmt = New-Object byte[] ([int]$len); [Array]::Copy($b, [int]$body, $fmt, 0, [int]$len) }
    elseif ($id -eq 'data') {
      $avail = [math]::Min($len, $b.Length - $body)
      $data = New-Object byte[] ([int]$avail)
      [Array]::Copy($b, [int]$body, $data, 0, [int]$avail)
      break
    }
    $pos = $body + $len + ($len % 2)
  }
  if ($null -eq $fmt -or $fmt.Length -lt 16 -or $null -eq $data -or $data.Length -eq 0) { throw "$Path has no fmt or data chunk" }
  $rate = [int64][BitConverter]::ToUInt32($fmt, 8)
  if ($rate -le 0) { throw "$Path has a zero byte rate" }
  [pscustomobject]@{ Fmt = $fmt; Data = $data; ByteRate = $rate; Seconds = ($data.Length / $rate) }
}

# Writes -Dest: the audio of -Source repeated as many whole times as it takes to reach -Seconds (at least once). Returns
# Path, Repeats, Seconds (the real length) and SourceSeconds. Whole repeats keep every sentence intact.
function New-BilingualLoop {
  param([Parameter(Mandatory = $true)] [string]$Source, [Parameter(Mandatory = $true)] [string]$Dest, [Parameter(Mandatory = $true)] [double]$Seconds)
  $w = Read-BilingualWav -Path $Source
  $reps = [int][math]::Max(1, [math]::Ceiling($Seconds / $w.Seconds - 1e-9))
  $asc = [Text.Encoding]::ASCII
  $dataLen = [int64]$w.Data.Length * $reps
  if ($dataLen -ge 4GB - 1MB) { throw 'the looped input would be larger than a WAV file can hold' }
  $pad = $w.Fmt.Length % 2
  $riff = 4L + 8 + $w.Fmt.Length + $pad + 8 + $dataLen
  $dir = Split-Path -Parent $Dest
  if ($dir) { New-Item -ItemType Directory -Force -Path $dir | Out-Null }
  $fs = [IO.File]::Create($Dest)
  try {
    $bw = New-Object IO.BinaryWriter($fs)
    $bw.Write($asc.GetBytes('RIFF')); $bw.Write([uint32]$riff); $bw.Write($asc.GetBytes('WAVE'))
    $bw.Write($asc.GetBytes('fmt ')); $bw.Write([uint32]$w.Fmt.Length); $bw.Write([byte[]]$w.Fmt)
    if ($pad -eq 1) { $bw.Write([byte]0) }
    $bw.Write($asc.GetBytes('data')); $bw.Write([uint32]$dataLen)
    for ($i = 0; $i -lt $reps; $i++) { $bw.Write([byte[]]$w.Data) }
    $bw.Flush()
  } finally { $fs.Dispose() }
  [pscustomobject]@{ Path = $Dest; Repeats = $reps; Seconds = ($w.Seconds * $reps); SourceSeconds = $w.Seconds }
}

Export-ModuleMember -Function Get-BilingualNightModels, Get-BilingualSettings, Get-BilingualCell, Get-BilingualDevice, Get-BilingualLoopSeconds, Get-BilingualPlan, New-BilingualRequest, Get-BilingualRunStats, Get-BilingualMedian, Get-BilingualPart, Format-BilingualNum, Format-BilingualPartText, Format-BilingualHeadline, Format-BilingualWhere, Get-BilingualReportLines, Read-BilingualWav, New-BilingualLoop
