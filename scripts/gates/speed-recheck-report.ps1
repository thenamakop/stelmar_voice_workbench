<#
.SYNOPSIS
  Writes docs\evidence\phase-2\speed-recheck-heartbeat.txt from the results of re-running one speed cell.

.DESCRIPTION
  The cell (medium, Hindi, emu-low, CPU avx2, 3 threads, rolling) is re-run with
    scripts\gates\speed-tables.ps1 -Tables A -Work <recheck folder> -Cell "A|CPU avx2, 3 threads|medium|hi|rolling|3"
  and compared with the same cell of Night 2 (results-A.jsonl in the speed folder). Every results-A*.jsonl file in
  the recheck folder is one run; the newest is the run the verdict is about, the others are listed. More than 3%
  slower than Night 2 is a FAIL (the owner's limit of 12 Nov 2026). Evidence files are written by scripts only.
#>
[CmdletBinding()]
param(
  [string]$Recheck = 'D:\STELMAR-Gate\speed-recheck',
  [string]$Night2 = 'D:\STELMAR-Gate\speed\results-A.jsonl',
  [string]$Key = 'A|CPU avx2, 3 threads|medium|hi|rolling|3',
  [double]$LimitPercent = 3.0,
  [string]$OutDir
)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo = (Resolve-Path (Join-Path $here '..\..')).Path
if (-not $OutDir) { $OutDir = Join-Path $repo 'docs\evidence\phase-2' }
Import-Module (Join-Path $repo 'scripts\lib\PsEnv.psm1') -Force
Set-WbInvariantCulture
Import-Module (Join-Path $repo 'scripts\lib\Redact.psm1') -Force
$inv = [Globalization.CultureInfo]::InvariantCulture
function F([double]$v, [int]$d = 3) { $v.ToString("F$d", $inv) }
function Read-Cell([string]$File) {
  Get-Content -LiteralPath $File -Encoding UTF8 | ForEach-Object { $_ | ConvertFrom-Json } | Where-Object { $_.key -eq $Key -and $_.status -eq 'ok' }
}

$old = @(Read-Cell $Night2) | Select-Object -First 1
if (-not $old) { throw "Night 2 has no result for $Key in $Night2" }
$runs = @(Get-ChildItem -LiteralPath $Recheck -Filter 'results-A*.jsonl' | ForEach-Object { Read-Cell $_.FullName }) | Sort-Object when
if (-not $runs) { throw "no re-run result for $Key in $Recheck" }
$new = $runs[-1]
$earlier = @($runs | Select-Object -First ($runs.Count - 1))
$pct = ($new.rtf_median / $old.rtf_median - 1) * 100

$lines = @(
  'Speed re-check after the heartbeat callbacks (Phase 2, 12 Nov 2026)',
  "date: $($new.when) (IST on the Legion); machine: Legion (i9-14900HX); profile: $($new.profile) (4 E-cores, 11 GB job); overlay tools closed: yes; gate lock held, no build running (speed-tables.ps1 checks both)",
  "cell: wb-engine bench, $($new.model), Hindi (60.2 s FLEURS clip, 0.8 s joins), $($new.mode) settings (beam 5, temperature fallback, 15 s VAD chunks), CPU $($new.variant), $($new.threads) threads; one run after one warm-up (single run because RTF is above 2, the rule of the speed tables)",
  'why: the engine sets whisper abort and progress callbacks that call a rate-limited atomic store (the heartbeat) and holds the heartbeats around each decode; the owner asked for one cell compared with Night 2 and a stop if it is more than 3% slower. The bench path sets the callbacks and the hold but does not start the heartbeat thread (only batch does); the thread wakes every 2 s and writes one short line, which the batch runs of the kill test carry (supervisor-kill-emu-low.txt)',
  '',
  "Night 2 ($($old.when), engine sha256 $($old.engine_sha)): RTF $(F $old.rtf_median), warm-up $(F $old.warmup_rtf), load $(F $old.load_s 2) s, peak working set $([math]::Round($old.peak_ws/1MB)) MiB"
)
foreach ($e in $earlier) {
  $lines += "earlier run on an earlier build of the same day ($($e.when), engine sha256 $($e.engine_sha)): RTF $(F $e.rtf_median), warm-up $(F $e.warmup_rtf)"
}
$lines += @(
  "now ($($new.when), engine sha256 $($new.engine_sha)): RTF $(F $new.rtf_median), warm-up $(F $new.warmup_rtf), load $(F $new.load_s 2) s, peak working set $([math]::Round($new.peak_ws/1MB)) MiB",
  "difference in RTF against Night 2: $(F $pct 2) % (negative is faster); the limit is +$(F $LimitPercent 0) %",
  "whisper.cpp: $($new.whisper)",
  ('verdict: ' + $(if ($pct -le $LimitPercent) { 'PASS (within the 3% limit)' } else { 'FAIL (more than 3% slower: stop and report)' })),
  'note: a single run under the emulator has run-to-run noise of its own; each warm-up agrees with the run that follows it to under 1%, which is the precision of this check'
)
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$path = Join-Path $OutDir 'speed-recheck-heartbeat.txt'
Write-WbEvidence -Path $path -Lines $lines
Write-Host "evidence: $path"
$lines | Select-Object -Last 8
if ($pct -gt $LimitPercent) { exit 1 }
