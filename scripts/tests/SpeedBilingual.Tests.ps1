# Tests for the speed-bilingual gate (scripts/lib/SpeedBilingual.psm1, scripts/gate-pack/gates/speed-bilingual.ps1):
# the cell chosen from capability.json, the plan, the request the queue would send, the numbers, the result line,
# the registry entry and the README. No engine is started.
# Run: powershell -NoProfile -ExecutionPolicy Bypass -File scripts/tests/SpeedBilingual.Tests.ps1
# ASCII only; Windows PowerShell 5.1.
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo = (Resolve-Path (Join-Path $here '..\..')).Path
Import-Module (Join-Path $here '..\lib\PackCore.psm1') -Force
Import-Module (Join-Path $here '..\lib\SpeedBilingual.psm1') -Force
$fails = 0
function Check([string]$Name, [bool]$Ok) {
  if ($Ok) { Write-Host "ok   $Name" } else { Write-Host "FAIL $Name"; $script:fails++ }
}
function Near($a, $b, [double]$tol = 1e-6) { ($null -ne $a) -and ([math]::Abs([double]$a - [double]$b) -le $tol) }

# A capability.json as the quick check writes it: Hindi rolling on turbo q5_0, English on medium; two cells for turbo
# (a slower CPU one and the graphics one the decision picked) and a live stand-in that must be ignored.
$cap = '{
 "decision": { "tier": "C", "tier_reasons": ["x"], "config": { "config": { "kind": "gpu", "gpu": 0 }, "label": "Intel graphics" },
  "languages": {
   "en": { "rolling": { "model": "medium", "rtf": 0.347 } },
   "hi": { "rolling": { "model": "large-v3-turbo-q5_0", "rtf": 0.5366666666666666 } },
   "ur": { "rolling": null } } },
 "measurements": [
  { "model": "medium", "lang": "en", "mode": "rolling", "config": { "kind": "gpu", "index": 0 }, "engine_build": "vulkan", "device_name": "UHD", "threads": 2, "rtf": 0.347, "live_standin": false },
  { "model": "large-v3-turbo-q5_0", "lang": "hi", "mode": "rolling", "config": { "kind": "cpu" }, "engine_build": "avx2", "device_name": "CPU", "threads": 3, "rtf": 0.9, "live_standin": false },
  { "model": "large-v3-turbo-q5_0", "lang": "hi", "mode": "rolling", "config": { "kind": "gpu", "index": 1 }, "engine_build": "vulkan", "device_name": "UHD", "threads": 2, "rtf": 0.5366666666666666, "live_standin": false },
  { "model": "large-v3-turbo-q5_0", "lang": "hi", "mode": "rolling", "config": { "kind": "gpu", "index": 1 }, "engine_build": "vulkan", "device_name": "UHD", "threads": 2, "rtf": 0.1, "live_standin": true }
 ] }' | ConvertFrom-Json

# --- the cell ---
$hi = Get-BilingualCell -Capability $cap -Lang 'hi'
$en = Get-BilingualCell -Capability $cap -Lang 'en'
Check 'the Hindi cell is the measurement the decision picked (its RTF), not the CPU one or the live stand-in' ($hi.Model -eq 'large-v3-turbo-q5_0' -and $hi.Variant -eq 'vulkan' -and $hi.DeviceKind -eq 'gpu' -and $hi.GpuIndex -eq 1 -and $hi.Threads -eq 2)
Check 'the English cell is its own rolling model' ($en.Model -eq 'medium' -and $en.Threads -eq 2)
Check 'a language with no rolling model has no cell' ($null -eq (Get-BilingualCell -Capability $cap -Lang 'ur'))
Check 'no capability, no cell' ($null -eq (Get-BilingualCell -Capability $null -Lang 'hi'))
$cap2 = $cap | ConvertTo-Json -Depth 10 | ConvertFrom-Json
$cap2.decision.languages.hi.rolling.rtf = 0.123   # no measurement has this RTF: the fastest rolling one of the model is taken
$hi2 = Get-BilingualCell -Capability $cap2 -Lang 'hi'
Check 'without an exact RTF match the fastest rolling measurement of that model is taken (never the live stand-in)' ($hi2.Rtf -eq 0.5366666666666666)
Check 'a cpu cell gives a cpu device; a gpu cell its index' ((Get-BilingualDevice -Cell $hi).kind -eq 'gpu' -and (Get-BilingualDevice -Cell $hi).index -eq 1 -and (Get-BilingualDevice -Cell ([pscustomobject]@{ DeviceKind = 'cpu'; GpuIndex = $null })).kind -eq 'cpu')

# --- the plan ---
$plan = @(Get-BilingualPlan -Hi $hi -En $en -Quick $true)
$keys = @($plan | ForEach-Object { $_.Key })
Check 'the plan has the four measurements of the spec, in order, plus English alone (it costs under a minute here)' (($keys -join ',') -eq 'rolling-fixed-hi,rolling-detect-en-hi,night-fixed-hi,night-detect-en-hi,rolling-fixed-en')
$a = $plan | Where-Object Key -eq 'rolling-fixed-hi'; $b = $plan | Where-Object Key -eq 'rolling-detect-en-hi'
$c = $plan | Where-Object Key -eq 'night-fixed-hi'; $d = $plan | Where-Object Key -eq 'night-detect-en-hi'
Check 'a) rolling, bench-hi, hi only, detection off' ($a.Clip -eq 'bench-hi.wav' -and ($a.Languages -join ',') -eq 'hi' -and -not $a.Detect -and $a.Role -eq 'rolling' -and $a.Models['hi'] -eq 'large-v3-turbo-q5_0')
Check 'b) rolling, bench-codeswitch, en+hi, detection on, the cell''s model per language' ($b.Clip -eq 'bench-codeswitch.wav' -and ($b.Languages -join ',') -eq 'en,hi' -and $b.Detect -and $b.Models['en'] -eq 'medium' -and $b.Models['hi'] -eq 'large-v3-turbo-q5_0')
Check 'c) night, bench-hi, hi only, large-v3 for Hindi' ($c.Role -eq 'night' -and -not $c.Detect -and $c.Models['hi'] -eq 'large-v3-q5_0' -and $c.Clip -eq 'bench-hi.wav')
Check 'd) night, bench-codeswitch, turbo for English and large-v3 for Hindi, detection on' ($d.Role -eq 'night' -and $d.Detect -and $d.Models['en'] -eq 'large-v3-turbo-q5_0' -and $d.Models['hi'] -eq 'large-v3-q5_0' -and $d.Clip -eq 'bench-codeswitch.wav')
Check 'quick: each measurement once' (@($plan | Where-Object { $_.Repeats -ne 1 }).Count -eq 0)
$planF = @(Get-BilingualPlan -Hi $hi -En $en -Quick $false)
Check 'full: each measurement twice' (@($planF | Where-Object { $_.Repeats -ne 2 }).Count -eq 0)
$enSlow = [pscustomobject]@{ Lang = 'en'; Model = 'large-v3-q5_0'; Rtf = 0.9; Variant = 'avx2'; DeviceKind = 'cpu'; GpuIndex = $null; Threads = 3 }
Check 'English alone is left out when it would not cost under a minute (RTF 0.9 x 60 s)' (@(Get-BilingualPlan -Hi $hi -En $enSlow -Quick $true | Where-Object Key -eq 'rolling-fixed-en').Count -eq 0)
Check 'without an English cell the rolling en+hi measurement is not planned' (@(Get-BilingualPlan -Hi $hi -En $null -Quick $true | Where-Object Key -eq 'rolling-detect-en-hi').Count -eq 0)
Check 'without a Hindi cell there is no plan' (@(Get-BilingualPlan -Hi $null -En $en).Count -eq 0)

# --- the request is the queue's ---
$paths = @{ 'large-v3-turbo-q5_0' = 'C:\p\turbo.bin'; 'medium' = 'C:\p\medium.bin'; 'large-v3-q5_0' = 'C:\p\l3.bin' }
$req = New-BilingualRequest -Measurement $b -Cell $hi -ClipPath 'C:\p\bench-codeswitch.wav' -ModelPaths $paths -VadPath 'C:\p\vad.bin'
$json = $req | ConvertTo-Json -Depth 8
$back = $json | ConvertFrom-Json
Check 'the request is a batch of a file with no units, from the start, with two languages and detection on' ($back.type -eq 'batch' -and $back.source.kind -eq 'file' -and $back.source.path -eq 'C:\p\bench-codeswitch.wav' -and @($back.units).Count -eq 0 -and $back.resume_from_ms -eq 0 -and @($back.languages).Count -eq 2 -and $back.detect_language -eq $true)
Check 'the request carries the cell''s device and threads and one model per language' ($back.device.kind -eq 'gpu' -and $back.device.index -eq 1 -and $back.threads -eq 2 -and $back.models_by_lang.en.id -eq 'medium' -and $back.models_by_lang.hi.path -eq 'C:\p\turbo.bin')
$reqA = (New-BilingualRequest -Measurement $a -Cell $hi -ClipPath 'x' -ModelPaths $paths -VadPath 'v') | ConvertTo-Json -Depth 8 | ConvertFrom-Json
Check 'a single-language request has detection off (the engine refuses several languages with it off)' ($reqA.detect_language -eq $false -and @($reqA.languages).Count -eq 1)
$s = $back.settings
Check 'the settings are Settings::default with the role: beam 5, best of 5, fallback on, context on, 15 s chunks, sentence-sized, 2 s minimum' ($s.role -eq 'rolling' -and $s.beam_size -eq 5 -and $s.best_of -eq 5 -and $s.temperature_fallback -eq $true -and $s.previous_text_context -eq $true -and $s.context_chars -eq 200 -and $s.max_chunk_s -eq 15 -and $s.sentence_split -eq $true -and $s.min_chunk_s -eq 2 -and $s.vad_threshold -eq 0.5 -and $s.vad_min_silence_ms -eq 100 -and $s.flash_attn -eq $true -and $s.timestamps -eq $false)
Check 'the night request has the night role' (((New-BilingualRequest -Measurement $d -Cell $hi -ClipPath 'x' -ModelPaths $paths -VadPath 'v') | ConvertTo-Json -Depth 8 | ConvertFrom-Json).settings.role -eq 'night')
# Every field of Settings that the engine requires (no serde default) is present, so the engine does not refuse the request.
$proto = [IO.File]::ReadAllText((Join-Path $repo 'crates\wb-protocol\src\lib.rs'))
$m = [regex]::Match($proto, 'pub struct Settings \{(.*?)\n\}', 'Singleline')
$required = New-Object System.Collections.Generic.List[string]
$prevDefault = $false
foreach ($line in ($m.Groups[1].Value -split "`n")) {
  $t = $line.Trim()
  if ($t -match '^#\[serde\(default') { $prevDefault = $true; continue }
  if ($t -match '^pub (\w+):') { if (-not $prevDefault) { $required.Add($Matches[1]) }; $prevDefault = $false; continue }
}
$have = @($s.PSObject.Properties | ForEach-Object { $_.Name })
Check ('the request has every field the engine requires in Settings (' + $required.Count + ' of them)') (($required.Count -ge 9) -and (@($required | Where-Object { $have -notcontains $_ }).Count -eq 0))

# --- what one run says ---
$ev = @(
  [pscustomobject]@{ event = 'hello'; whisper_cpp = [pscustomobject]@{ version = '1.8.3'; commit = 'abc' } },
  [pscustomobject]@{ event = 'ready'; device_name = 'UHD'; load_s = 4.5 },
  [pscustomobject]@{ event = 'segment'; text = 'x' },
  [pscustomobject]@{ event = 'stats'; unit = 0; chunks = 9; audio_ms = 60000; decode_s = 60.66; rtf = 1.011; peak_working_set_bytes = 123; detections = 14; both_ways = 0 },
  [pscustomobject]@{ event = 'done'; segments = 9 })
$st = Get-BilingualRunStats -Events $ev
Check 'a finished run gives RTF, detections, chunks and the load time, and notes whisper.cpp' ($st.ok -and (Near $st.rtf 1.011) -and $st.detections -eq 14 -and $st.chunks -eq 9 -and (Near $st.load_s 4.5) -and $st.whisper -match '1\.8\.3' -and $st.segments -eq 1)
Check 'a run without a done event is not ok' (-not (Get-BilingualRunStats -Events @($ev | Where-Object { $_.event -ne 'done' })).ok)
$bad = Get-BilingualRunStats -Events @([pscustomobject]@{ event = 'error'; code = 'model'; message = 'out of memory' })
Check 'an error event is reported with its code and message' (-not $bad.ok -and $bad.error -match 'model: out of memory')
Check 'no events at all is not ok' (-not (Get-BilingualRunStats -Events @()).ok)

# --- the numbers: the owner's example, emu-low, 5-minute rolling unit 0.753 -> 1.011 ---
function Run($rtf, $audioS, $det) { [pscustomobject]@{ ok = $true; rtf = $rtf; decode_s = $rtf * $audioS; audio_ms = $audioS * 1000; detections = $det; chunks = 9; error = '' } }
$part = Get-BilingualPart -FixedRuns @((Run 0.753 60 0)) -DetectRuns @((Run 1.011 60 14))
Check 'slowdown factor is detect RTF over fixed RTF (1.011 / 0.753 = x1.34)' ($part.Ok -and (Near $part.Factor 1.3426 0.001))
Check 'detections, per minute and the extra seconds per detection ((1.011 - 0.753) x 60 s / 14)' ($part.Detections -eq 14 -and (Near $part.DetectionsPerMin 14) -and (Near $part.ExtraPerDetection ((1.011 - 0.753) * 60 / 14) 1e-6))
$night = Get-BilingualPart -FixedRuns @((Run 1.498 60 0)) -DetectRuns @((Run 1.868 60 14))
Check 'night: 1.498 -> 1.868 is x1.25' ((Near $night.Factor 1.2470 0.001))
$two = Get-BilingualPart -FixedRuns @((Run 0.70 60 0), (Run 0.80 60 0)) -DetectRuns @((Run 1.00 60 14), (Run 1.10 60 14))
Check 'two runs: the median of both (0.75 and 1.05)' ((Near $two.FixedRtf 0.75) -and (Near $two.DetectRtf 1.05))
$three = Get-BilingualMedian -Values @(3, 1, 2)
Check 'median of three, of two, of none' ($three -eq 2 -and (Get-BilingualMedian -Values @(1, 2)) -eq 1.5 -and $null -eq (Get-BilingualMedian -Values @()))
$none = Get-BilingualPart -FixedRuns @((Run 0.75 60 0)) -DetectRuns @([pscustomobject]@{ ok = $false; error = 'x' })
Check 'a part without a complete pair of good runs is not measured' (-not $none.Ok)
$zero = Get-BilingualPart -FixedRuns @((Run 0.75 60 0)) -DetectRuns @((Run 0.80 60 0))
Check 'no detections: no division by zero, extra seconds per detection is empty' ($zero.Ok -and $null -eq $zero.ExtraPerDetection)

# --- the result line ---
$head = Format-BilingualHeadline -Rolling $part -Night $night -Where (Format-BilingualWhere -Hi $hi -En $en)
Check 'the result line reads like the spec''s example' ($head -match '^rolling RTF 0\.75 fixed-hi / 1\.01 en\+hi with detection \(x1\.34; 14 detections, 1\.1 s each\); night 1\.50 / 1\.87 \(x1\.25\); vulkan gpu 1, hi large-v3-turbo-q5_0 / en medium, 2 threads$')
$head2 = Format-BilingualHeadline -Rolling $part -Night $none -Where 'w' -NotMeasured @('night-detect-en-hi: out of memory')
$fast = Get-BilingualPart -FixedRuns @((Run 0.84 60 0)) -DetectRuns @((Run 0.80 60 4))
$headFast = Format-BilingualHeadline -Rolling $fast -Night $night -Where 'w'
Check 'a detect run that is not slower than the fixed RTF predicts (the clips differ): factor shown, no negative time per detection in the line, the number kept in the evidence' ($fast.ExtraPerDetection -lt 0 -and $headFast -match 'x0\.95; 4 detections, no extra time seen per detection' -and $headFast -notmatch '-\d')
Check 'a part that could not be measured says so and why' ($head2 -match 'night not measured' -and $head2 -match 'out of memory')

# --- the evidence text: counts and numbers, no transcript ---
$rec = @([pscustomobject]@{ key = 'rolling-fixed-hi'; label = 'x'; models = 'hi=turbo'; clip = 'bench-hi.wav'; languages = 'hi'; detect = $false; runs = @((Run 0.70 60 0), (Run 0.80 60 0)) })
$lines = @(Get-BilingualReportLines -Records $rec -Rolling $part -Night $night -Machine 'M1' -Profile 'laptop' -Quick $false -Hi $hi -En $en -Notes @('a note'))
$text = $lines -join "`n"
Check 'the evidence lists each run, the median, the slowdown, the estimate caveat and says it is information only' ($text -match 'run 1: RTF 0\.700' -and $text -match 'run 2: RTF 0\.800' -and $text -match 'median RTF: 0\.750' -and $text -match 'slowdown x1\.34' -and $text -match 'estimate' -and $text -match 'information only: no verdict' -and $text -match 'note: a note')
Check 'the evidence says which mode (full: twice)' ($text -match 'full \(each measurement twice')

# --- the registry, the script, the README ---
$mf = Read-PackManifest -Path (Join-Path $repo 'scripts\gate-pack\gates.manifest.json')
$g = @($mf.gates | Where-Object { $_.id -eq 'speed-bilingual' })
Check 'the registry has speed-bilingual: P2, unattended, after quick-check, no ledger row (run rows only)' ($g.Count -eq 1 -and $g[0].priority -eq 'P2' -and -not $g[0].interactive -and @($g[0].depends_on) -contains 'quick-check' -and @($g[0].ledger).Count -eq 0)
Check 'the registry states the quick and the full estimate (8 and 15 minutes on the low laptop)' ($g[0].quick.minutes -eq 8 -and $g[0].est_minutes.low -eq 15 -and $g[0].est_minutes.mid -ge 8)
Check 'the registry says information only, no bar' ($g[0].threshold -match 'information only' -and $g[0].threshold -match 'no bar')
$sb = [IO.File]::ReadAllText((Join-Path $repo 'scripts\gate-pack\gates\speed-bilingual.ps1'))
$mod = [IO.File]::ReadAllText((Join-Path $repo 'scripts\lib\SpeedBilingual.psm1'))
Check 'the script and the module are ASCII only' (-not ($sb.ToCharArray() | Where-Object { [int]$_ -gt 126 }) -and -not ($mod.ToCharArray() | Where-Object { [int]$_ -gt 126 }))
Check 'the script reads the quick check''s kept capability.json and hard-codes no model, device or thread count for the rolling cell' ($sb -match "gates\\quick-check\\capability\.json" -and $sb -notmatch "'(small|medium)'" -and $sb -notmatch 'Threads\s*=\s*\d')
Check 'the script writes one JSON per measurement into the evidence' ($sb -match "measurement-\{0\}\.json")
Check 'the script runs the engine''s batch command with the queue-style request file and does not change an engine' ($sb -match "'batch', '--request'" -and $sb -notmatch 'cargo')
Check 'the script hands plain arrays (ToArray) to Get-BilingualPart, never @() around a generic List (that failed with "Argument types do not match" in the dry run of 18 Nov 2026)' ($sb -match 'RunsOf' -and $sb -notmatch "-FixedRuns @\(\`$runs\[" -and $sb -notmatch "-DetectRuns @\(\`$runs\[")
# The same call as the gate makes, on lists built the way the gate builds them.
$lst = New-Object System.Collections.Generic.List[object]; $lst.Add((Run 0.8 60 0))
$hh = @{ 'k' = $lst }
function RunsOf2([string]$Key) { if ($hh.ContainsKey($Key)) { return , @($hh[$Key].ToArray()) } else { return , @() } }
Check 'a part is computed from runs kept in generic lists, via ToArray' ((Get-BilingualPart -FixedRuns (RunsOf2 'k') -DetectRuns (RunsOf2 'k')).Ok)
Check 'a measurement that was never planned gives an empty pair, not an error' (-not (Get-BilingualPart -FixedRuns (RunsOf2 'k') -DetectRuns (RunsOf2 'missing')).Ok)
Check 'the script takes the gate lock like the other measuring gates' ($sb -match "Invoke-WbGate -Name 'speed-bilingual'")
Check 'no transcript text is kept: the engine output file is overwritten per run and only counts are written' ($sb -notmatch '\.text\b' -and $mod -notmatch '\.text\b')
$readme = [IO.File]::ReadAllText((Join-Path $repo 'scripts\gate-pack\README.txt'))
Check 'the README mentions the gate with its quick and full time' ($readme -match 'language detection' -and $readme -match 'about\s+8\s+minutes' -and $readme -match 'about\s+15\s+minutes')

if ($fails -gt 0) { Write-Host "$fails failed"; exit 1 }
Write-Host 'SpeedBilingual tests passed'
