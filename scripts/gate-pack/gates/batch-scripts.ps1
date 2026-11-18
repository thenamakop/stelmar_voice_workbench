# Gate batch-scripts: three public FLEURS clips per language (English, Hindi, Urdu) go through the medium model
# with wb-engine batch, on the CPU and on the integrated graphics adapter the vulkan engine lists. A run
# passes when the engine exits 0, emits a done event and at least one segment, the text has no U+FFFD and
# no run of question marks, and at least 90% of its letters are in the language's script. The evidence holds
# counts only, never transcript text. Clips: data\batch-clips\<lang>\<id>.wav (public FLEURS test, CC BY 4.0).
# Windows PowerShell 5.1, ASCII only.
param([string]$CtxFile, [string]$ResultFile)
$ErrorActionPreference = 'Stop'
$lib = Join-Path $PSScriptRoot '..\..\lib'
Import-Module (Join-Path $lib 'PackCore.psm1') -Force
Import-Module (Join-Path $lib 'PackGate.psm1') -Force
$ctx = Read-PackCtx -CtxFile $CtxFile
Initialize-PackGate -Ctx $ctx
Import-Module (Join-Path $lib 'SpeedTable.psm1') -Force
Import-Module (Join-Path $lib 'RecorderGates.psm1') -Force
$paths = Get-PackPaths -Ctx $ctx

Invoke-PackGateBody -Ctx $ctx -Id 'batch-scripts' -ResultFile $ResultFile -Body {
  $quick = [bool]$ctx.quick
  $profile = [string]$ctx.profile
  $perLang = $(if ($quick) { 1 } else { 3 })
  $thr = 'every run exits 0 with a done event and a segment; at least 90% of the letters in the language''s script; no broken text'
  $triple = 'x86_64-pc-windows-msvc'
  $work = Join-Path ([string]$ctx.gate_dir) 'work'
  New-Item -ItemType Directory -Force -Path $work | Out-Null
  $med = Join-Path $paths.Models 'ggml-medium.bin'
  if (-not (Test-Path -LiteralPath $med)) { throw "the medium model is missing from the pack: $med" }
  $langs = @(
    @{ Lang = 'en'; Script = 'Latin'; Pattern = '[A-Za-z]' },
    @{ Lang = 'hi'; Script = 'Devanagari'; Pattern = '[\u0900-\u097F]' },
    @{ Lang = 'ur'; Script = 'Arabic script'; Pattern = '[\u0600-\u06FF\u0750-\u077F\uFB50-\uFDFF\uFE70-\uFEFF]' })
  $threads = [math]::Max(2, [math]::Min(12, [int]([Environment]::ProcessorCount)))
  if ($profile -ne 'laptop') {
    # Under wb-emulate the engine sees only the emulated processors: ask it (info), as the speed tables do.
    try {
      $ae = Join-Path $paths.Engines "wb-engine-avx2-$triple.exe"
      $ri = Invoke-PackNative -FilePath $paths.Emulate -ArgumentList @('--profile', $profile, '--', $ae, 'info') -TimeoutSeconds 60
      foreach ($l in @($ri.Output)) { if ($l -match '^\{') { $j = $l | ConvertFrom-Json; if ($j.PSObject.Properties['event'] -and $j.event -eq 'info' -and [int]$j.logical_processors_allowed -ge 1) { $threads = [int]$j.logical_processors_allowed } } }
    } catch { if ($profile -eq 'emu-low') { $threads = 4 } }
  }
  $devs = New-Object System.Collections.Generic.List[object]
  $devs.Add(@{ Name = 'CPU (avx2)'; Variant = 'avx2'; Arg = 'cpu' })
  $vk = Join-Path $paths.Engines "wb-engine-vulkan-$triple.exe"
  $gpuNote = 'no integrated graphics adapter listed by the vulkan engine'
  if (Test-Path -LiteralPath $vk) {
    $all = @(Get-SpeedDevices -Exe $vk -ProcName "wb-engine-vulkan-$triple" -WorkDir $work -Emulate $paths.Emulate -Profile $profile)
    $igp = @($all | Where-Object { $_.Backend -match 'Vulkan' -and $_.Kind -match 'integrated|igpu' } | Select-Object -First 1)
    if ($igp.Count -eq 0) { $igp = @($all | Where-Object { $_.Backend -match 'Vulkan' -and $_.Kind -ne 'cpu' } | Select-Object -First 1) }
    if ($igp.Count -gt 0) { $devs.Add(@{ Name = ('{0} (vulkan device {1}, driver {2})' -f $igp[0].Description, $igp[0].Index, $igp[0].Driver); Variant = 'vulkan'; Arg = ('gpu:{0}' -f $igp[0].Index) }); $gpuNote = $igp[0].Description }
  }
  $lines = New-Object System.Collections.Generic.List[string]
  $lines.Add('Batch check: public FLEURS clips per language through medium, right script (gate pack)')
  $lines.Add(('machine: {0}; profile: {1}; clips per language: {2}; model: medium; threads: {3}; date: {4}' -f $ctx.machine, $profile, $perLang, $threads, (Get-Date -Format 'yyyy-MM-dd HH:mm')))
  $lines.Add('pass rule: exit 0, a done event, at least one segment, no U+FFFD, no run of ? characters, at least 90% of letters and combining marks in the language''s script; counts only, no transcript text')
  $problems = New-Object System.Collections.Generic.List[string]
  $total = 0
  $skipped = New-Object System.Collections.Generic.List[string]
  foreach ($dev in $devs) {
    $exe = Join-Path $paths.Engines ("wb-engine-{0}-{1}.exe" -f $dev.Variant, $triple)
    if (-not (Test-Path -LiteralPath $exe)) { $skipped.Add("engine $($dev.Variant) is not in the pack"); continue }
    $lines.Add(('== {0}' -f $dev.Name))
    foreach ($l in $langs) {
      $dir = Join-Path $paths.Data ('batch-clips\' + $l.Lang)
      $clips = @(Get-ChildItem -LiteralPath $dir -Filter '*.wav' -File -ErrorAction SilentlyContinue | Sort-Object Name | Select-Object -First $perLang)
      if ($clips.Count -eq 0) { $skipped.Add("no clips for $($l.Lang) in the pack"); continue }
      foreach ($c in $clips) {
        $total++
        $out = Join-Path $work ('batch-{0}-{1}-{2}.jsonl' -f $dev.Variant, $l.Lang, $c.BaseName)
        if (Test-Path -LiteralPath $out) { Remove-Item -LiteralPath $out -Force }
        $ea = @('batch', '--file', ('"' + $c.FullName + '"'), '--models', ('{0}=medium' -f $l.Lang), '--langs', $l.Lang, '--out', ('"' + $out + '"'), '--threads', "$threads", '--device', $dev.Arg, '--models-dir', ('"' + $paths.Models + '"'))
        $r = Invoke-SpeedEngine -Exe $exe -ProcName ("wb-engine-{0}-{1}" -f $dev.Variant, $triple) -EngineArgs $ea -TimeoutMin 30 -WorkDir $work -Emulate $paths.Emulate -Profile $profile
        $ev = @()
        if (Test-Path -LiteralPath $out) { $ev = @(Read-SpeedEvents @(Get-Content -LiteralPath $out -Encoding UTF8)) }
        $segs = @($ev | Where-Object { $_.event -eq 'segment' })
        $done = $ev | Where-Object { $_.event -eq 'done' } | Select-Object -First 1
        $text = ($segs | ForEach-Object { [string]$_.text }) -join ' '
        $letters = [regex]::Matches($text, '[\p{L}\p{M}]').Count
        $inScript = [regex]::Matches($text, '(?=[\p{L}\p{M}])' + $l.Pattern).Count
        $share = $(if ($letters) { $inScript / $letters } else { 0 })
        $bad = ($text.Contains([string][char]0xFFFD)) -or ($text -match '\?{3,}')
        $ok = ($r.Exit -eq 0) -and $done -and ($segs.Count -ge 1) -and (-not $bad) -and ($share -ge 0.9)
        if (-not $ok) { $problems.Add(('{0}/{1}/{2}' -f $dev.Variant, $l.Lang, $c.BaseName)) }
        $lines.Add(('{0} | {1} | {2} | exit {3} | segments {4} | letters and marks {5} | {6} share {7}% | broken text {8} | {9} s | {10}' -f $dev.Variant, $l.Lang, $c.BaseName, $r.Exit, $segs.Count, $letters, $l.Script, [math]::Round(100 * $share, 1).ToString([Globalization.CultureInfo]::InvariantCulture), $(if ($bad) { 'YES' } else { 'no' }), $r.Seconds, $(if ($ok) { 'ok' } else { 'WRONG' })))
        if (-not $ok) { foreach ($e in @($r.Err | Select-Object -Last 3)) { if ("$e".Trim()) { $lines.Add("   stderr: $("$e".Trim())") } } }
      }
    }
  }
  $lines.Add(('runs: {0}; problems: {1}' -f $total, $problems.Count))
  foreach ($p in $problems) { $lines.Add("   problem: $p") }
  foreach ($s in ($skipped | Sort-Object -Unique)) { $lines.Add("   not run: $s") }
  $status = 'PASS'
  if ($total -eq 0) { $status = 'INVALID' }
  elseif ($problems.Count -gt 0) { $status = 'FAIL' }
  elseif ($skipped.Count -gt 0) { $status = 'INVALID' }
  $lines.Add('verdict: ' + $status)
  $ev = Join-Path ([string]$ctx.gate_dir) 'batch-scripts.txt'
  Set-Content -LiteralPath $ev -Value @($lines) -Encoding UTF8
  $head = ('{0} runs on {1} device(s) ({2}), {3} problem(s)' -f $total, $devs.Count, $gpuNote, $problems.Count)
  if ($skipped.Count -gt 0) { $head += '; some runs could not be made (' + ((@($skipped | Sort-Object -Unique)) -join '; ') + ')' }
  New-PackResult -Id 'batch-scripts' -Status $status -Headline $head -Measured ([ordered]@{ runs = $total; problems = $problems.Count; devices = @($devs | ForEach-Object { $_.Name }); clips_per_language = $perLang }) -Threshold $thr -Evidence @($ev) -Notes @($skipped | Sort-Object -Unique) -Quick $quick
}
