# Samples what a laptop's processor is doing while a gate runs: clock, performance limit, thermal-zone
# temperature, mains or battery, and (when nvidia-smi exists) the graphics card's temperature and
# clock. One CSV row every -IntervalSec seconds until the stop file appears. Counters that Windows
# does not expose on this machine are left empty, never written as zero.
# Not pinned to any core; normal priority. Windows PowerShell 5.1, ASCII only.
param(
  [Parameter(Mandatory = $true)] [string]$OutCsv,
  [Parameter(Mandatory = $true)] [string]$StopFile,
  [int]$IntervalSec = 5
)
$ErrorActionPreference = 'Continue'
$inv = [System.Globalization.CultureInfo]::InvariantCulture
[System.Threading.Thread]::CurrentThread.CurrentCulture = $inv

$paths = [ordered]@{
  cpu_pct      = '\Processor Information(_Total)\% Processor Time'
  perf_pct     = '\Processor Information(_Total)\% Processor Performance'
  freq_mhz     = '\Processor Information(_Total)\Processor Frequency'
  perf_limit   = '\Processor Information(_Total)\% Performance Limit'
  temp         = '\Thermal Zone Information(*)\Temperature'
  passive      = '\Thermal Zone Information(*)\% Passive Limit'
}
# Which counters this machine has: tried once each.
$have = @{}
foreach ($k in $paths.Keys) {
  try { $null = Get-Counter -Counter $paths[$k] -ErrorAction Stop; $have[$k] = $true } catch { $have[$k] = $false }
}
# "\\host\processor information(_total)\% processor time" -> "processor information|% processor time"
$sampleKey = {
  param([string]$p)
  if ($p -match '\\([^\\(]+)\([^)]*\)\\([^\\]+)$') { return ($Matches[1].ToLowerInvariant() + '|' + $Matches[2].ToLowerInvariant()) }
  ''
}
$keyOf = @{}
foreach ($k in $paths.Keys) { $keyOf[$k] = (& $sampleKey $paths[$k]) }
$usable = @($paths.Keys | Where-Object { $have[$_] } | ForEach-Object { $paths[$_] })
$nvidia = $null
try { $nvidia = (Get-Command nvidia-smi.exe -ErrorAction Stop).Source } catch { }

$header = 'time_utc,elapsed_s,mains,cpu_pct,perf_pct,freq_mhz,perf_limit_pct,temp_c_max,passive_limit_pct_min,gpu_temp_c,gpu_clock_mhz'
Set-Content -LiteralPath $OutCsv -Value $header -Encoding ASCII
$start = Get-Date
function F($v) { if ($null -eq $v) { '' } else { ([double]$v).ToString('0.##', $inv) } }

while (-not (Test-Path -LiteralPath $StopFile)) {
  $tick = Get-Date
  $row = @{}
  # One call for every counter this machine has (a call takes about a second whatever it asks for).
  $samples = @()
  if ($usable.Count -gt 0) {
    try { $samples = @((Get-Counter -Counter $usable -ErrorAction Stop).CounterSamples) } catch { $samples = @() }
  }
  foreach ($k in $paths.Keys) {
    if (-not $have[$k]) { continue }
    $want = ($keyOf[$k])
    $vals = @($samples | Where-Object { (& $sampleKey $_.Path) -eq $want } | ForEach-Object { [double]$_.CookedValue })
    if ($vals.Count -eq 0) { continue }
    if ($k -eq 'temp') {
      $m = ($vals | Measure-Object -Maximum).Maximum
      $row[$k] = $(if ($m -gt 200) { $m - 273.15 } else { $m })
    } elseif ($k -eq 'passive') { $row[$k] = ($vals | Measure-Object -Minimum).Minimum }
    else { $row[$k] = $vals[0] }
  }
  $mains = ''
  try {
    $b = @(Get-CimInstance Win32_Battery -ErrorAction Stop)
    if ($b.Count -gt 0) { $mains = $(if (@($b | Where-Object { [int]$_.BatteryStatus -eq 1 }).Count -gt 0) { '0' } else { '1' }) }
  } catch { }
  $gt = $null; $gc = $null
  if ($nvidia) {
    try {
      $o = (& $nvidia '--query-gpu=temperature.gpu,clocks.gr' '--format=csv,noheader,nounits' 2>$null | Select-Object -First 1)
      if ($o -match '^\s*(\d+)\s*,\s*(\d+)') { $gt = [double]$Matches[1]; $gc = [double]$Matches[2] }
    } catch { }
  }
  $line = '{0},{1},{2},{3},{4},{5},{6},{7},{8},{9},{10}' -f $tick.ToUniversalTime().ToString('o'), (F (($tick - $start).TotalSeconds)), $mains, (F $row['cpu_pct']), (F $row['perf_pct']), (F $row['freq_mhz']), (F $row['perf_limit']), (F $row['temp']), (F $row['passive']), (F $gt), (F $gc)
  try { Add-Content -LiteralPath $OutCsv -Value $line -Encoding ASCII } catch { }
  $wait = $IntervalSec - ((Get-Date) - $tick).TotalSeconds
  if ($wait -gt 0) { Start-Sleep -Milliseconds ([int]($wait * 1000)) }
}
