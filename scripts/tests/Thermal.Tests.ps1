# Tests for scripts/lib/Thermal.psm1: the summary of synthetic runs (throttling, clean, battery,
# counters not exposed) and a short real run of the sampler.
# Run: powershell -NoProfile -ExecutionPolicy Bypass -File scripts/tests/Thermal.Tests.ps1
# ASCII only; Windows PowerShell 5.1.
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
Import-Module (Join-Path $here '..\lib\Thermal.psm1') -Force
$fails = 0
function Check([string]$Name, [bool]$Ok) {
  if ($Ok) { Write-Host "ok   $Name" } else { Write-Host "FAIL $Name"; $script:fails++ }
}
$tmp = Join-Path ([IO.Path]::GetTempPath()) ('wb-thermal-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path $tmp | Out-Null
$hdr = 'time_utc,elapsed_s,mains,cpu_pct,perf_pct,freq_mhz,perf_limit_pct,temp_c_max,passive_limit_pct_min,gpu_temp_c,gpu_clock_mhz'

function New-Csv([string]$name, [scriptblock]$rowOf, [int]$seconds, [int]$step = 5) {
  $p = Join-Path $tmp $name
  $lines = @($hdr)
  for ($t = 0; $t -le $seconds; $t += $step) { $lines += (& $rowOf $t) }
  Set-Content -LiteralPath $p -Value $lines -Encoding ASCII
  $p
}

try {
  # A run whose clock falls from 3000 to 2000 MHz and whose limit drops below 100 in the second half.
  $thr = New-Csv 'throttle.csv' { param($t) $f = $(if ($t -lt 3600) { 3000 } else { 2000 }); $lim = $(if ($t -lt 3600) { 100 } else { 70 }); "x,$t,1,60,100,$f,$lim,$(60 + [math]::Min(30, $t / 200)),100,,"  } 7200
  $s = Get-PackThermalSummary -Rows (ConvertTo-PackThermalRows -Csv $thr)
  Check 'a long run is usable' ($s.Usable -and $s.Samples -eq 1441)
  Check 'the windows are 10 minutes' ($s.WindowS -eq 600)
  Check 'the first and last median clock are read' ($s.Freq.FirstMedian -eq 3000 -and $s.Freq.LastMedian -eq 2000 -and $s.Freq.Min -eq 2000)
  Check 'a clock drop of a third is reported as throttling' ([math]::Round($s.ClockDropPct) -eq 33 -and (@($s.Notes) -join ' ') -match 'throttling is likely')
  Check 'samples below the performance limit are counted' ($s.LimitBelow100 -gt 0 -and $s.LimitBelow100 -lt $s.LimitSamples)
  Check 'the highest temperature is read' ($s.Temp.Exposed -and $s.Temp.Max -eq 90)
  Check 'mains in every sample is not an AC loss' (-not $s.AcLost -and $s.MainsKnown -eq $s.Samples)

  # A clean short run (the window is a third of it).
  $clean = New-Csv 'clean.csv' { param($t) "x,$t,1,50,100,3200,100,70,100,,"  } 600
  $s2 = Get-PackThermalSummary -Rows (ConvertTo-PackThermalRows -Csv $clean)
  Check 'a short run uses a third of its length as the window' ($s2.WindowS -eq 200)
  Check 'a steady clock has no drop and no throttling note' ($s2.ClockDropPct -eq 0 -and -not ((@($s2.Notes) -join ' ') -match 'throttling'))

  # Battery during the run.
  $bat = New-Csv 'battery.csv' { param($t) $m = $(if ($t -ge 300 -and $t -lt 330) { 0 } else { 1 }); "x,$t,$m,50,100,3200,100,70,100,,"  } 600
  $s3 = Get-PackThermalSummary -Rows (ConvertTo-PackThermalRows -Csv $bat)
  Check 'a stretch on battery is an AC loss' ($s3.AcLost -and $s3.AcLostSamples -eq 6)
  Check 'the AC loss shows in the text' ((@(Get-PackThermalLines -Summary $s3) -join "`n") -match 'ON BATTERY')

  # Counters that are not exposed, and a desktop without a battery.
  $none = New-Csv 'none.csv' { param($t) "x,$t,,50,,,,,,," } 300
  $s4 = Get-PackThermalSummary -Rows (ConvertTo-PackThermalRows -Csv $none)
  Check 'a missing clock is "not exposed", not zero' (-not $s4.Freq.Exposed -and (@($s4.Notes) -join ' ') -match 'CPU clock: not exposed')
  Check 'a missing temperature is "not exposed"' (-not $s4.Temp.Exposed -and (@($s4.Notes) -join ' ') -match 'temperature: not exposed')
  Check 'no battery is said plainly and is not an AC loss' ((@($s4.Notes) -join ' ') -match 'no battery' -and -not $s4.AcLost)
  $few = New-Csv 'few.csv' { param($t) "x,$t,1,50,100,3200,100,70,100,," } 5
  Check 'two samples are too few' (-not (Get-PackThermalSummary -Rows (ConvertTo-PackThermalRows -Csv $few)).Usable)
  Check 'the median of an even count is the mean of the middle two' ((Get-PackMedian @(1, 2, 3, 10)) -eq 2.5)

  # The real sampler, 12 seconds.
  $csv = Join-Path $tmp 'real.csv'
  $h = Start-PackThermal -OutCsv $csv -IntervalSec 2
  Start-Sleep -Seconds 22
  $r = Stop-PackThermal -Handle $h
  Check 'the real sampler stopped on its stop file' ($h.Process.HasExited)
  Check 'the real sampler wrote a header and several rows' ((@(Get-Content -LiteralPath $csv)).Count -ge 4 -and (Get-Content -LiteralPath $csv -TotalCount 1) -eq $hdr)
  Check 'the real summary is usable' ($r.Summary.Usable)
  Check 'the real sampler line text is not empty' (@($r.Lines).Count -ge 2)
  Write-Host ('info this machine: CPU clock exposed {0}; temperature exposed {1}; event 37 count {2}' -f $r.Summary.Freq.Exposed, $r.Summary.Temp.Exposed, $r.Event37)
} finally {
  Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue
}
if ($fails) { Write-Host "$fails failed"; exit 1 }
Write-Host 'Thermal tests passed'
