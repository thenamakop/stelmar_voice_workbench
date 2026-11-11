<#
.SYNOPSIS
  Phase 2 batch check: three FLEURS clips per language, decoded by the medium model through wb-engine batch,
  come out in the right script, on the CPU and on the integrated GPU, under emu-low and emu-mid.

.DESCRIPTION
  For each profile (emu-low, emu-mid) and each device (CPU with the avx2 engine; the Intel UHD with the
  vulkan engine, device 0 as listed by `info` inside the profile): runs `wb-engine batch --file <clip>
  --models <lang>=medium --langs <lang>` on the first three clips of each of fleurs-en, fleurs-hi and
  fleurs-ur in eval\clips\phase2-verify-fleurs.txt (the 20 clips per language that Phase 5a scored).
  A run passes when the engine exits 0, emits a `done` event and at least one segment, the text holds no
  U+FFFD and no run of question marks, and at least 90% of its letters are in the language's script (Latin
  for en, Devanagari for hi, Arabic-script for ur). The evidence holds counts only, no transcript text.
  Not timing-sensitive (no gate lock), but nothing else heavy should run. About 15 minutes.
  Evidence: docs\evidence\phase-2\batch-scripts.txt
#>
[CmdletBinding()]
param(
  [string[]]$Profiles = @('emu-low', 'emu-mid'),
  [int]$ClipsPerLanguage = 3,
  [string]$Model = 'medium',
  [string]$Machine = 'Legion',
  [string]$Work = 'D:\STELMAR-Gate\batch-scripts',
  [string]$Out
)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo = (Resolve-Path (Join-Path $here '..\..')).Path
if (-not $Out) { $Out = Join-Path $repo 'docs\evidence\phase-2\batch-scripts.txt' }
Import-Module (Join-Path $repo 'scripts\lib\PsEnv.psm1') -Force
Set-WbCleanPSModulePath
Set-WbInvariantCulture
Import-Module (Join-Path $repo 'scripts\lib\GateRun.psm1') -Force
Import-Module (Join-Path $repo 'scripts\lib\Redact.psm1') -Force
$inv = [Globalization.CultureInfo]::InvariantCulture
$triple = 'x86_64-pc-windows-msvc'
$emulate = Join-Path $repo 'target\release\wb-emulate.exe'
$modelsDir = Join-Path $repo 'models'
New-Item -ItemType Directory -Force -Path $Work | Out-Null

$overlay = @(Get-WbOverlayTools)
$builds = @(Get-WbRunningBuilds)
if ($overlay.Count -or $builds.Count) { throw "refusing to run: overlay tools ($($overlay -join ', ')) or builds ($($builds -join ', ')) are running" }

# The clips: the first N ids per dataset in the committed clip list.
$ids = @{}
foreach ($line in (Get-Content (Join-Path $repo 'eval\clips\phase2-verify-fleurs.txt') -Encoding UTF8)) {
  if ($line.StartsWith('#') -or -not $line.Trim()) { continue }
  $p = $line -split "`t"
  if (-not $ids.ContainsKey($p[0])) { $ids[$p[0]] = @() }
  if ($ids[$p[0]].Count -lt $ClipsPerLanguage) { $ids[$p[0]] += $p[1] }
}
$langs = @(@{ Lang = 'en'; Set = 'fleurs-en'; Script = 'Latin'; Pattern = '[A-Za-z]' },
           @{ Lang = 'hi'; Set = 'fleurs-hi'; Script = 'Devanagari'; Pattern = '[\u0900-\u097F]' },
           @{ Lang = 'ur'; Set = 'fleurs-ur'; Script = 'Arabic script'; Pattern = '[\u0600-\u06FF\u0750-\u077F\uFB50-\uFDFF\uFE70-\uFEFF]' })

function Get-Events([string]$File) {
  if (-not (Test-Path $File)) { return @() }
  $text = [IO.File]::ReadAllText($File, [Text.Encoding]::UTF8)
  @($text -split "`r?`n" | Where-Object { $_.StartsWith('{') } | ForEach-Object { try { $_ | ConvertFrom-Json } catch { $null } } | Where-Object { $_ })
}

$lines = @(
  'Batch check: three FLEURS clips per language through medium, right script, on the CPU and on the UHD (Phase 2)',
  "date: $((Get-Date).ToString('yyyy-MM-dd HH:mm', $inv)) (IST on the Legion); machine: $Machine; model: $Model; clips: the first $ClipsPerLanguage per language of eval/clips/phase2-verify-fleurs.txt (FLEURS test, ids only below; no text)",
  'engines: avx2 (CPU rows) and vulkan (UHD rows) copied from src-tauri\binaries; run through wb-emulate; overlay tools closed: yes; no build running: yes',
  'pass rule: exit 0, a done event, at least one segment, no U+FFFD, no run of ? characters, at least 90% of the letters and combining marks in the language''s script',
  ''
)
$problems = @()
$total = 0
foreach ($p in $Profiles) {
  $threads = $(if ($p -eq 'emu-low') { 4 } else { 12 })
  $lines += "== profile $p ($threads threads)"
  foreach ($dev in @(@{ Name = 'CPU (avx2)'; Variant = 'avx2'; Arg = 'cpu' }, @{ Name = 'Intel UHD (vulkan, device 0)'; Variant = 'vulkan'; Arg = 'gpu:0' })) {
    $exe = Join-Path $repo "src-tauri\binaries\wb-engine-$($dev.Variant)-$triple.exe"
    if (-not (Test-Path $exe)) { throw "engine not built: $exe" }
    foreach ($l in $langs) {
      foreach ($id in $ids[$l.Set]) {
        $total++
        $wav = Join-Path $repo "eval\data\$($l.Set)\audio\$id.wav"
        $outFile = Join-Path $Work "$p-$($dev.Variant)-$($l.Lang)-$id.jsonl"
        if (Test-Path $outFile) { Remove-Item $outFile -Force }
        $argList = @('--profile', $p, '--', ('"' + $exe + '"'), 'batch', '--file', ('"' + $wav + '"'), '--models', "$($l.Lang)=$Model", '--langs', $l.Lang,
                     '--out', ('"' + $outFile + '"'), '--threads', "$threads", '--device', $dev.Arg, '--models-dir', ('"' + $modelsDir + '"'))
        $watch = [Diagnostics.Stopwatch]::StartNew()
        $r = Invoke-WbNative -FilePath $emulate -ArgumentList $argList
        $watch.Stop()
        $ev = Get-Events $outFile
        $segs = @($ev | Where-Object { $_.event -eq 'segment' })
        $done = $ev | Where-Object { $_.event -eq 'done' } | Select-Object -First 1
        $text = ($segs | ForEach-Object { [string]$_.text }) -join ' '
        $letters = [regex]::Matches($text, '[\p{L}\p{M}]').Count  # letters and combining marks (Devanagari vowel signs are marks)
        # Only letters and combining marks count (the Devanagari danda U+0964 is punctuation inside the script's block).
        $inScript = [regex]::Matches($text, '(?=[\p{L}\p{M}])' + $l.Pattern).Count
        $share = $(if ($letters) { $inScript / $letters } else { 0 })
        $bad = ($text.Contains([string][char]0xFFFD)) -or ($text -match '\?{3,}')
        $ok = ($r.ExitCode -eq 0) -and $done -and ($segs.Count -ge 1) -and (-not $bad) -and ($share -ge 0.9)
        if (-not $ok) { $problems += "$p/$($dev.Name)/$($l.Lang)/$id" }
        $wall = $(if ($done -and [double]$done.audio_ms -gt 0) { [math]::Round($watch.Elapsed.TotalSeconds * 1000 / [double]$done.audio_ms, 2).ToString($inv) } else { 'n/a' })
        $lines += "$($dev.Name) | $($l.Lang) | $id | exit $($r.ExitCode) | segments $($segs.Count) | letters and marks $letters | $($l.Script) share $([math]::Round(100 * $share, 1).ToString($inv))% | broken text $(if ($bad) { 'YES' } else { 'no' }) | wall time over audio $wall (with load) | $(if ($ok) { 'ok' } else { 'WRONG' })"
        if (-not $ok) { foreach ($e in ($r.Error | Select-Object -Last 3)) { if ("$e".Trim()) { $lines += "   stderr: $("$e".Trim())" } } }
      }
    }
  }
  $lines += ''
}
$lines += "runs: $total; problems: $($problems.Count)"
foreach ($pr in $problems) { $lines += "   problem: $pr" }
$lines += $(if ($problems.Count -eq 0) { 'verdict: PASS (PROVISIONAL: emulated)' } else { 'verdict: FAIL' })
New-Item -ItemType Directory -Force -Path (Split-Path $Out) | Out-Null
Write-WbEvidence -Path $Out -Lines $lines -MachineLabel $Machine
Write-Host "evidence: $Out"
$lines | Select-Object -Last 3
if ($problems.Count) { exit 1 }
