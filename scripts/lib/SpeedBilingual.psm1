<#
  SpeedBilingual.psm1: the pure parts of the speed-bilingual gate (information only, no bar).

  Question: how much does per-chunk language detection slow real queue work on THIS laptop? The system check
  benches one language with no detection; the queue of a two-language session detects the language of every
  chunk. Four measurements, each on the engine's own `batch` command with the request the queue builds
  (crates/wb-runner/src/exec.rs build_request: Settings::default with the role, sentence-sized chunks, VAD
  chunking, detection on when the session has more than one language):
    a) rolling, bench-hi.wav, language hi only, no detection (the conditions of the quick check's cell);
    b) rolling, bench-codeswitch.wav, languages en+hi, detection on every chunk;
    c) night, as a), with large-v3 for Hindi;
    d) night, as b), turbo for English and large-v3 for Hindi (the two models the queue's night engine loads).
  Optional e) rolling, bench-en.wav, English only, when it costs under a minute.
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

# Pure. The measurements to make. -Hi and -En are cells from Get-BilingualCell. All run on the Hindi cell's engine,
# device and threads (one engine holds both models, as the queue starts it).
# Returns objects: Key, Part (rolling|night|en), Mode (fixed|detect), Clip, Role, Languages, Models (lang -> id), Detect, Repeats, Label.
function Get-BilingualPlan {
  param($Hi, $En, [bool]$Quick = $false, [double]$ClipSeconds = 60)
  $reps = $(if ($Quick) { 1 } else { 2 })
  $plan = New-Object System.Collections.Generic.List[object]
  if ($null -eq $Hi) { return @() }
  $plan.Add([pscustomobject]@{ Key = 'rolling-fixed-hi'; Part = 'rolling'; Mode = 'fixed'; Clip = 'bench-hi.wav'; Role = 'rolling'; Languages = @('hi'); Models = [ordered]@{ hi = $Hi.Model }; Detect = $false; Repeats = $reps
      Label = 'rolling, bench-hi, language fixed to hi, no detection' })
  if ($En) {
    $plan.Add([pscustomobject]@{ Key = 'rolling-detect-en-hi'; Part = 'rolling'; Mode = 'detect'; Clip = 'bench-codeswitch.wav'; Role = 'rolling'; Languages = @('en', 'hi'); Models = [ordered]@{ en = $En.Model; hi = $Hi.Model }; Detect = $true; Repeats = $reps
        Label = 'rolling, bench-codeswitch, languages en+hi, detection on every chunk' })
  }
  $nm = Get-BilingualNightModels
  $plan.Add([pscustomobject]@{ Key = 'night-fixed-hi'; Part = 'night'; Mode = 'fixed'; Clip = 'bench-hi.wav'; Role = 'night'; Languages = @('hi'); Models = [ordered]@{ hi = $nm['hi'] }; Detect = $false; Repeats = $reps
      Label = 'night, bench-hi, language fixed to hi, no detection' })
  $plan.Add([pscustomobject]@{ Key = 'night-detect-en-hi'; Part = 'night'; Mode = 'detect'; Clip = 'bench-codeswitch.wav'; Role = 'night'; Languages = @('en', 'hi'); Models = [ordered]@{ en = $nm['en']; hi = $nm['hi'] }; Detect = $true; Repeats = $reps
      Label = 'night, bench-codeswitch, languages en+hi (turbo for en, large-v3 for hi), detection on every chunk' })
  # English alone: only when it costs under a minute (the cell's own RTF times the clip, plus a load allowance).
  if ($En -and ($En.Rtf * $ClipSeconds) -le 40) {
    $plan.Add([pscustomobject]@{ Key = 'rolling-fixed-en'; Part = 'en'; Mode = 'fixed'; Clip = 'bench-en.wav'; Role = 'rolling'; Languages = @('en'); Models = [ordered]@{ en = $En.Model }; Detect = $false; Repeats = $reps
        Label = 'rolling, bench-en, language fixed to en, no detection' })
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

# Pure. One part (rolling or night) from the runs of its fixed and its detect measurement.
# -FixedRuns and -DetectRuns are arrays of Get-BilingualRunStats objects.
# ExtraPerDetection: the seconds the detect run took beyond what the fixed run's RTF predicts for its audio,
# per detection. It assumes the same speed on both clips, so it is an estimate, and says so in the evidence.
function Get-BilingualPart {
  param($FixedRuns, $DetectRuns)
  $f = @($FixedRuns | Where-Object { $_ -and $_.ok })
  $d = @($DetectRuns | Where-Object { $_ -and $_.ok })
  if ($f.Count -eq 0 -or $d.Count -eq 0) { return [pscustomobject]@{ Ok = $false; FixedRtf = $null; DetectRtf = $null; Factor = $null; Detections = $null; DetectionsPerMin = $null; ExtraPerDetection = $null; FixedRuns = $f.Count; DetectRuns = $d.Count } }
  $fr = Get-BilingualMedian -Values @($f | ForEach-Object { [double]$_.rtf })
  $dr = Get-BilingualMedian -Values @($d | ForEach-Object { [double]$_.rtf })
  $det = [int][math]::Round((Get-BilingualMedian -Values @($d | ForEach-Object { [double]$_.detections })))
  $audioS = (Get-BilingualMedian -Values @($d | ForEach-Object { [double]$_.audio_ms })) / 1000.0
  $dec = Get-BilingualMedian -Values @($d | ForEach-Object { [double]$_.decode_s })
  $factor = $null
  if ($fr -gt 0) { $factor = $dr / $fr }
  $perMin = $null
  if ($audioS -gt 0) { $perMin = $det / ($audioS / 60.0) }
  $extra = $null
  if ($det -gt 0) { $extra = ($dec - $fr * $audioS) / $det }
  [pscustomobject]@{ Ok = $true; FixedRtf = $fr; DetectRtf = $dr; Factor = $factor; Detections = $det; DetectionsPerMin = $perMin; ExtraPerDetection = $extra; FixedRuns = $f.Count; DetectRuns = $d.Count }
}

function Format-BilingualNum {
  param($Value, [string]$Format = '0.00')
  if ($null -eq $Value) { return 'n/a' }
  ([double]$Value).ToString($Format, $script:Inv)
}

# Pure. The result line: "rolling RTF 0.75 fixed-hi / 1.01 en+hi with detection (x1.34; 14 detections, 1.1 s each);
# night 1.50 / 1.87 (x1.25); vulkan gpu 0, hi medium / en small, 2 threads".
function Format-BilingualHeadline {
  param($Rolling, $Night, [string]$Where, [string[]]$NotMeasured = @())
  $parts = New-Object System.Collections.Generic.List[string]
  if ($Rolling -and $Rolling.Ok) {
    $each = ''
    if ($null -ne $Rolling.ExtraPerDetection) { $each = ', {0} s each' -f (Format-BilingualNum $Rolling.ExtraPerDetection '0.0') }
    $parts.Add(('rolling RTF {0} fixed-hi / {1} en+hi with detection (x{2}; {3} detections{4})' -f (Format-BilingualNum $Rolling.FixedRtf), (Format-BilingualNum $Rolling.DetectRtf), (Format-BilingualNum $Rolling.Factor), $Rolling.Detections, $each))
  } else { $parts.Add('rolling not measured') }
  if ($Night -and $Night.Ok) { $parts.Add(('night {0} / {1} (x{2})' -f (Format-BilingualNum $Night.FixedRtf), (Format-BilingualNum $Night.DetectRtf), (Format-BilingualNum $Night.Factor))) } else { $parts.Add('night not measured') }
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
# -Records: one per measurement with key, label, models, clip, languages, detect, runs (Get-BilingualRunStats objects).
function Get-BilingualReportLines {
  param($Records, $Rolling, $Night, [string]$Machine, [string]$Profile, [bool]$Quick, $Hi, $En, [string[]]$Notes = @())
  $L = New-Object System.Collections.Generic.List[string]
  $mode = 'full (each measurement twice, median reported)'
  if ($Quick) { $mode = 'quick (each measurement once)' }
  $L.Add('Speed with and without per-chunk language detection (gate pack, information only, no bar)')
  $L.Add(('machine: {0}; profile: {1}; mode: {2}; date: {3}' -f $Machine, $Profile, $mode, (Get-Date).ToString('yyyy-MM-dd HH:mm', $script:Inv)))
  if ($Hi) { $L.Add(('cell chosen by this laptop''s quick check: {0}' -f (Format-BilingualWhere -Hi $Hi -En $En))) }
  $L.Add('request: the queue''s (Settings::default with the role, beam 5, best of 5, sentence-sized chunks, detection on when two languages)')
  $L.Add('RTF here = engine seconds for the unit (VAD, detection, decoding; not model load) / audio seconds')
  $L.Add('')
  foreach ($r in $Records) {
    $L.Add(('{0}: {1}' -f $r.key, $r.label))
    $det = 'off'
    if ($r.detect) { $det = 'on' }
    $L.Add(('   models: {0}; clip: {1}; languages: {2}; detection: {3}' -f $r.models, $r.clip, $r.languages, $det))
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
      $L.Add(('{0}: fixed-language RTF {1}; with detection {2}; slowdown x{3}; {4} detections ({5} per minute of audio); about {6} s extra per detection' -f $p[0], (Format-BilingualNum $x.FixedRtf '0.000'), (Format-BilingualNum $x.DetectRtf '0.000'), (Format-BilingualNum $x.Factor), $x.Detections, (Format-BilingualNum $x.DetectionsPerMin '0.0'), (Format-BilingualNum $x.ExtraPerDetection '0.0')))
    } else { $L.Add(('{0}: not measured (no complete pair of runs)' -f $p[0])) }
  }
  $L.Add('seconds per detection is an estimate: (engine seconds of the detect run - fixed RTF x its audio seconds) / detections; it assumes the same speed on the two clips (bench-hi and bench-codeswitch differ).')
  foreach ($n in $Notes) { $L.Add('note: ' + $n) }
  $L.Add('information only: no verdict')
  @($L.ToArray())
}

Export-ModuleMember -Function Get-BilingualNightModels, Get-BilingualSettings, Get-BilingualCell, Get-BilingualDevice, Get-BilingualPlan, New-BilingualRequest, Get-BilingualRunStats, Get-BilingualMedian, Get-BilingualPart, Format-BilingualNum, Format-BilingualHeadline, Format-BilingualWhere, Get-BilingualReportLines
