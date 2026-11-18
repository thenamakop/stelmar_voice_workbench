# Tests for scripts/lib/Hashing.psm1: model_hash lines, the 0.6 rule, raw and warm read rates.
# Run: powershell -NoProfile -ExecutionPolicy Bypass -File scripts/tests/Hashing.Tests.ps1
# ASCII only; Windows PowerShell 5.1.
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
Import-Module (Join-Path $here '..\lib\Hashing.psm1') -Force
$fails = 0
function Check([string]$Name, [bool]$Ok) {
  if ($Ok) { Write-Host "ok   $Name" } else { Write-Host "FAIL $Name"; $script:fails++ }
}
$lines = @(
  '2026-11-14 model_hash file=ggml-small.bin bytes=488000000 start_ms=1000 end_ms=3000 took_ms=2000 mb_per_s=244 result=ok background=true memory_priority=1',
  '2026-11-14 model_hash file=ggml-medium.bin bytes=1500000000 start_ms=3000 end_ms=9000 took_ms=6000 mb_per_s=250 result=ok background=true memory_priority=1',
  '2026-11-14 model_hash file=bad.bin bytes=100 start_ms=0 end_ms=1 took_ms=1 mb_per_s=1 result=mismatch background=true memory_priority=1',
  'some other line')
$rows = @(ConvertFrom-PackHashLines -Lines $lines)
Check 'three model_hash lines are read' ($rows.Count -eq 3 -and $rows[0].File -eq 'ggml-small.bin' -and $rows[0].Background)
$rate = Get-PackHashRate -Rows $rows
Check 'the rate counts only files that hashed ok: bytes over seconds' ([math]::Abs($rate - (1988.0 / 8.0)) -lt 0.01)
Check 'no rows give no rate' ($null -eq (Get-PackHashRate -Rows @()))

$ev = @('  ggml-small.bin | 488000000 | 1,000 | 2000 | 244 | ok | True | 1', '  ggml-medium.bin | 1500000000 | 3,000 | 6000 | 250 | ok | True | 1', '  file | bytes | start | took ms | MB/s | result | background | memory priority')
$er = @(ConvertFrom-PackHashEvidence -Lines $ev)
Check 'the evidence table is read like the log lines' ($er.Count -eq 2 -and $er[1].TookMs -eq 6000 -and $er[0].Background)

$c = Test-PackColdRun -ColdMBs 120 -WarmMBs 250
Check 'a rate at 0.48 of the warm rate counts as cold' ($c.Counts -and [math]::Abs($c.Ratio - 0.48) -lt 0.001 -and $c.Reason -match 'not proof')
$c = Test-PackColdRun -ColdMBs 150 -WarmMBs 250
Check 'exactly 0.6 of the warm rate still counts' ($c.Counts)
$c = Test-PackColdRun -ColdMBs 200 -WarmMBs 250
Check 'a rate above 0.6 of the warm rate does not count and says it cannot be shown to be cold' (-not $c.Counts -and $c.Reason -match 'cannot be shown to be cold')
Check 'no warm rate means it cannot be called cold' (-not (Test-PackColdRun -ColdMBs 100 -WarmMBs $null).Counts)
Check 'no cold rate means it cannot be called cold' (-not (Test-PackColdRun -ColdMBs $null -WarmMBs 250).Counts)

function Run([string]$Tag, [string]$Kind, [string]$Status, $Rate, [bool]$Purged = $true) { [pscustomobject]@{ Tag = $Tag; Kind = $Kind; Status = $Status; Rate = $Rate; Purged = $Purged; Shared = 0; AppOnly = 0 } }
$warmRuns = @((Run 'w6' 'warm' 'PASS' 250), (Run 'w7' 'warm' 'PASS' 260), (Run 'w8' 'warm' 'PASS' 240))
$coldRuns = @((Run 'c2' 'purged' 'PASS' 100), (Run 'c3' 'purged' 'PASS' 120), (Run 'c4' 'purged' 'PASS' 140), (Run 'c5' 'purged' 'PASS' 90))
$v = Get-PackHashingVerdict -Runs (@($coldRuns) + @($warmRuns))
Check 'four purged runs at most 0.6 of the median warm rate (250) all count: PASS' ($v.Status -eq 'PASS' -and $v.ColdCount -eq 4 -and [math]::Abs($v.WarmRate - 250) -lt 0.01)
$mix = @($coldRuns[0], (Run 'c3' 'purged' 'PASS' 240), $coldRuns[2], $coldRuns[3]) + @($warmRuns)
$v = Get-PackHashingVerdict -Runs $mix
Check 'a purged run read at warm speed does not count, so the cold claim is INFO with the count' ($v.Status -eq 'INFO' -and $v.ColdCount -eq 3 -and $v.Headline -match 'only 3 of the 4')
$v = Get-PackHashingVerdict -Runs (@($coldRuns[0], (Run 'c3' 'purged' 'PASS' 100 $false)) + @($warmRuns)) -NeedCold 1
Check 'a purge that did not happen never counts as cold' ($v.ColdCount -eq 1 -and ($v.Rows | Where-Object { $_.Tag -eq 'c3' }).Note -match 'could not be emptied')
$v = Get-PackHashingVerdict -Runs (@($coldRuns[0], (Run 'w9' 'warm' 'FAIL' 250)) + @($warmRuns))
Check 'any failing run fails the gate' ($v.Status -eq 'FAIL' -and $v.Headline -match 'w9')
$v = Get-PackHashingVerdict -Runs (@($coldRuns[0], (Run 'w9' 'warm' 'INVALID' $null)) + @($warmRuns))
Check 'an invalid run, with none failing, makes the gate invalid' ($v.Status -eq 'INVALID')
$v = Get-PackHashingVerdict -Runs @($coldRuns[0]) -NeedCold 1
Check 'without any warm run no purged run can be called cold' ($v.ColdCount -eq 0 -and $v.Status -eq 'INFO')

$tmp = Join-Path ([IO.Path]::GetTempPath()) ('wb-hash-' + [guid]::NewGuid().ToString('N') + '.bin')
try {
  $bytes = New-Object byte[] (40MB)
  (New-Object Random 7).NextBytes($bytes)
  [IO.File]::WriteAllBytes($tmp, $bytes)
  $r = Measure-PackReadRate -Path $tmp -Bytes 16MB
  Check 'a raw read gives a positive rate' ($null -ne $r -and $r -gt 0)
  $w = Measure-PackWarmHashRate -Path $tmp -Bytes 16MB
  Check 'a warm hash gives a positive rate' ($null -ne $w -and $w -gt 0)
} finally { Remove-Item -LiteralPath $tmp -Force -ErrorAction SilentlyContinue }
$admin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $admin) { Check 'without administrator rights the purge says no instead of throwing' (-not (Clear-PackStandbyList)) }
else { Write-Host 'skip purge check (running as administrator: it would really empty the cache)' }

if ($fails) { Write-Host "$fails failed"; exit 1 }
Write-Host 'Hashing tests passed'
