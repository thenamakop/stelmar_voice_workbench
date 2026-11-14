# Tests for scripts/lib/ProcWatch.psm1: turning watcher lines into bench cells, and a real watch of a
# process whose name starts with the watched prefix (a copy of a harmless program named wb-engine-test).
# Run: powershell -NoProfile -ExecutionPolicy Bypass -File scripts/tests/ProcWatch.Tests.ps1
# ASCII only; Windows PowerShell 5.1.
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
Import-Module (Join-Path $here '..\lib\ProcWatch.psm1') -Force
$fails = 0
function Check([string]$Name, [bool]$Ok) {
  if ($Ok) { Write-Host "ok   $Name" } else { Write-Host "FAIL $Name"; $script:fails++ }
}

$req1 = '{"bench":{"model":{"id":"small","file":"ggml-small.bin"},"vad_model":{"id":"silero"},"lang":"hi","clip":"C:\\x\\bench-hi.wav","mode":"rolling","device":{"kind":"gpu","index":0},"threads":4,"runs":3}}'
$line1 = (@{ pid = 11; exe = 'wb-engine-vulkan.exe'; command = 'bench'; started = '2026-11-14T10:00:00.0000000Z'; ended = '2026-11-14T10:00:42.5000000Z'; request = $req1 } | ConvertTo-Json -Compress)
$line2 = (@{ pid = 12; exe = 'wb-engine-avx2.exe'; command = 'bench'; started = '2026-11-14T10:01:00.0000000Z'; ended = '2026-11-14T10:03:00.0000000Z'; request = '{"model":"medium","lang":"en","device":"cpu","threads":2,"mode":"near_live","clip":"bench-en.wav"}' } | ConvertTo-Json -Compress)
$line3 = (@{ pid = 13; exe = 'wb-engine-avx2.exe'; command = 'info'; started = '2026-11-14T10:00:00.0000000Z'; ended = '2026-11-14T10:00:01.0000000Z'; request = '' } | ConvertTo-Json -Compress)
$cells = @(Get-PackBenchCells -Lines @($line2, $line1, 'garbage', $line3))
Check 'garbage lines are skipped and the rest are in start order' ($cells.Count -eq 3 -and $cells[2].Pid -eq 12)
$c1 = $cells | Where-Object { $_.Pid -eq 11 }
Check 'the seconds are end minus start' ($c1.Seconds -eq 42.5)
Check 'the nested request fields are found' ($c1.Model -eq 'small' -and $c1.Lang -eq 'hi' -and $c1.Threads -eq 4 -and $c1.Mode -eq 'rolling' -and $c1.Clip -eq 'bench-hi.wav')
Check 'a device object is described by its kind' ($c1.Device -eq 'gpu')
$c2 = $cells | Where-Object { $_.Pid -eq 12 }
Check 'a flat request is read too' ($c2.Model -eq 'medium' -and $c2.Device -eq 'cpu' -and $c2.Threads -eq 2)
$sum = Get-PackCellSummary -Cells $cells -CheckSeconds 400
Check 'the summary counts only bench cells' ($sum.Cells -eq 2 -and [math]::Abs($sum.TotalSeconds - 162.5) -lt 0.01)
Check 'the longest cell is named' ($sum.LongestSeconds -eq 120 -and $sum.Longest -match 'medium en')
Check 'the cover is the share of the check' ([math]::Abs($sum.CoverPct - 40.625) -lt 0.01)

# A real watch.
$tmp = Join-Path ([IO.Path]::GetTempPath()) ('wb-procwatch-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path $tmp | Out-Null
try {
  $fake = Join-Path $tmp 'wb-engine-test.exe'
  Copy-Item -LiteralPath (Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe') -Destination $fake
  $sleeper = Join-Path $tmp 'sleeper.ps1'
  Set-Content -LiteralPath $sleeper -Value 'Start-Sleep -Seconds 6' -Encoding ascii
  $reqFile = Join-Path $tmp 'bench-request-1.json'
  Set-Content -LiteralPath $reqFile -Value $req1 -Encoding UTF8
  $h = Start-PackProcWatch -Dir (Join-Path $tmp 'watch')
  Start-Sleep -Seconds 2
  $p = Start-Process -FilePath $fake -ArgumentList @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', ('"{0}"' -f $sleeper), 'bench', '--request', ('"{0}"' -f $reqFile)) -PassThru -WindowStyle Hidden
  $null = $p.Handle
  Start-Sleep -Seconds 3
  Remove-Item -LiteralPath $reqFile -Force
  [void]$p.WaitForExit(15000)
  Start-Sleep -Seconds 1
  $got = @(Stop-PackProcWatch -Handle $h)
  $mine = @($got | Where-Object { $_.Pid -eq $p.Id })
  Check 'the real watcher saw the process' ($mine.Count -eq 1)
  Check 'it copied the request file before the app deleted it' ($mine.Count -eq 1 -and $mine[0].Model -eq 'small' -and $mine[0].Lang -eq 'hi')
  Check 'it measured how long the process lived' ($mine.Count -eq 1 -and $mine[0].Seconds -ge 2 -and $mine[0].Seconds -le 14)
} finally { Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue }

if ($fails) { Write-Host "$fails failed"; exit 1 }
Write-Host 'ProcWatch tests passed'
