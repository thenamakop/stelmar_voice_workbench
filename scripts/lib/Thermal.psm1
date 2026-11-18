<#
  Thermal.psm1: thermal and power evidence for the long gates (the 2-hour soak above all).

  Start-PackThermal starts scripts/lib/thermal-sampler.ps1 as its own process (CPU clock, performance
  limit, thermal-zone temperature, mains or battery, graphics card temperature when nvidia-smi
  exists). Stop-PackThermal stops it and returns the summary. Get-PackThermalSummary is the pure part:
  it reads the CSV and says what the numbers show. Counters that Windows does not expose on a machine
  are reported as "not exposed", never as zero.

  The summary is information, not a verdict, except for one thing: any moment on battery during a
  gate that needs mains power makes that gate INVALID (AcLost).

  Windows PowerShell 5.1, ASCII only.
#>

function Get-PackMedian {
  param([double[]]$Values)
  $v = @($Values | Sort-Object)
  if ($v.Count -eq 0) { return $null }
  $m = [int][math]::Floor($v.Count / 2)
  if ($v.Count % 2 -eq 1) { return [double]$v[$m] }
  ([double]$v[$m - 1] + [double]$v[$m]) / 2.0
}

function ConvertTo-PackThermalRows {
  param([Parameter(Mandatory = $true)] [string]$Csv)
  $inv = [System.Globalization.CultureInfo]::InvariantCulture
  $num = {
    param($s)
    if ($null -eq $s -or "$s" -eq '') { return $null }
    $d = 0.0
    if ([double]::TryParse([string]$s, [System.Globalization.NumberStyles]::Float, $inv, [ref]$d)) { return $d }
    $null
  }
  $rows = New-Object System.Collections.Generic.List[object]
  foreach ($r in @(Import-Csv -LiteralPath $Csv)) {
    $rows.Add([pscustomobject]@{
        Elapsed = (& $num $r.elapsed_s); Mains = $(if ("$($r.mains)" -eq '') { $null } else { [int]$r.mains })
        Cpu = (& $num $r.cpu_pct); Perf = (& $num $r.perf_pct); Freq = (& $num $r.freq_mhz); Limit = (& $num $r.perf_limit_pct)
        Temp = (& $num $r.temp_c_max); Passive = (& $num $r.passive_limit_pct_min); GpuTemp = (& $num $r.gpu_temp_c); GpuClock = (& $num $r.gpu_clock_mhz)
      })
  }
  @($rows.ToArray())
}

function Get-PackSeriesStat {
  param($Rows, [string]$Prop, [double]$Window)
  $all = @($Rows | Where-Object { $null -ne $_.$Prop })
  if ($all.Count -eq 0) { return [pscustomobject]@{ Exposed = $false } }
  $dur = [double](($Rows | ForEach-Object { $_.Elapsed } | Measure-Object -Maximum).Maximum)
  $first = @($all | Where-Object { $_.Elapsed -le $Window } | ForEach-Object { [double]$_.$Prop })
  $last = @($all | Where-Object { $_.Elapsed -ge ($dur - $Window) } | ForEach-Object { [double]$_.$Prop })
  $vals = @($all | ForEach-Object { [double]$_.$Prop })
  [pscustomobject]@{
    Exposed = $true
    FirstMedian = (Get-PackMedian $first); LastMedian = (Get-PackMedian $last)
    Min = ($vals | Measure-Object -Minimum).Minimum; Max = ($vals | Measure-Object -Maximum).Maximum
    Samples = $all.Count
  }
}

# Pure. Rows from ConvertTo-PackThermalRows. The first and last windows are 10 minutes, or a third of
# the run when it is shorter than half an hour.
function Get-PackThermalSummary {
  [CmdletBinding()]
  param([Parameter(Mandatory = $true)] $Rows, [int]$IntervalSec = 5)
  $rows = @($Rows)
  if ($rows.Count -lt 3) { return [pscustomobject]@{ Samples = $rows.Count; Usable = $false; Notes = @('too few samples for a summary') } }
  $dur = [double](($rows | ForEach-Object { $_.Elapsed } | Measure-Object -Maximum).Maximum)
  $window = [math]::Min(600.0, $dur / 3.0)
  $freq = Get-PackSeriesStat -Rows $rows -Prop 'Freq' -Window $window
  $perf = Get-PackSeriesStat -Rows $rows -Prop 'Perf' -Window $window
  $temp = Get-PackSeriesStat -Rows $rows -Prop 'Temp' -Window $window
  $gpuT = Get-PackSeriesStat -Rows $rows -Prop 'GpuTemp' -Window $window
  $limit = @($rows | Where-Object { $null -ne $_.Limit })
  $passive = @($rows | Where-Object { $null -ne $_.Passive })
  $mainsKnown = @($rows | Where-Object { $null -ne $_.Mains })
  $acLost = @($mainsKnown | Where-Object { $_.Mains -eq 0 })
  $notes = New-Object System.Collections.Generic.List[string]
  $dropPct = $null
  if ($freq.Exposed -and $freq.FirstMedian -gt 0) {
    $dropPct = 100.0 * ($freq.FirstMedian - $freq.LastMedian) / $freq.FirstMedian
    if ($dropPct -gt 15) { $notes.Add(('the median clock fell {0:n0}% from the first to the last {1:n0} minutes: thermal or power throttling is likely' -f $dropPct, ($window / 60.0))) }
  }
  $throttled = $null
  if ($limit.Count -gt 0) {
    $throttled = @($limit | Where-Object { $_.Limit -lt 100 }).Count
    if ($throttled -gt 0) { $notes.Add(('Windows reported a performance limit below 100% in {0} of {1} samples' -f $throttled, $limit.Count)) }
  }
  foreach ($pair in @(@('Freq', 'CPU clock', $freq), @('Temp', 'thermal-zone temperature', $temp), @('Limit', 'performance limit', $null))) {
    $has = @($rows | Where-Object { $null -ne $_.($pair[0]) }).Count -gt 0
    if (-not $has) { $notes.Add("$($pair[1]): not exposed by Windows on this machine") }
  }
  if ($mainsKnown.Count -eq 0) { $notes.Add('mains or battery: no battery on this machine (a desktop), or Windows does not report it') }
  [pscustomobject]@{
    Usable = $true; Samples = $rows.Count; DurationS = $dur; WindowS = $window
    Freq = $freq; Perf = $perf; Temp = $temp; GpuTemp = $gpuT
    ClockDropPct = $dropPct
    LimitSamples = $limit.Count; LimitBelow100 = $throttled
    PassiveSamples = $passive.Count; PassiveBelow100 = $(if ($passive.Count -gt 0) { @($passive | Where-Object { $_.Passive -lt 100 }).Count } else { $null })
    MainsKnown = $mainsKnown.Count; AcLostSamples = $acLost.Count; AcLost = ($acLost.Count -gt 0)
    Notes = @($notes.ToArray())
  }
}

# Text lines for evidence and for the gate's result.
function Get-PackThermalLines {
  [CmdletBinding()]
  param($Summary, [int]$Event37 = -1)
  if ($null -eq $Summary -or -not $Summary.Usable) { return @('  thermal: ' + $(if ($Summary) { ($Summary.Notes -join '; ') } else { 'not sampled' })) }
  $f = $Summary.Freq; $t = $Summary.Temp
  $lines = @()
  $lines += ('  thermal: {0} samples over {1:n0} min; first and last window {2:n0} min each' -f $Summary.Samples, ($Summary.DurationS / 60.0), ($Summary.WindowS / 60.0))
  if ($f.Exposed) { $lines += ('  thermal: CPU clock median {0:n0} MHz at the start, {1:n0} MHz at the end, lowest {2:n0} MHz{3}' -f $f.FirstMedian, $f.LastMedian, $f.Min, $(if ($null -ne $Summary.ClockDropPct) { (' (change {0:n1}%)' -f (-1.0 * $Summary.ClockDropPct)) } else { '' })) }
  if ($Summary.Perf.Exposed) { $lines += ('  thermal: processor performance median {0:n0}% at the start, {1:n0}% at the end, lowest {2:n0}%' -f $Summary.Perf.FirstMedian, $Summary.Perf.LastMedian, $Summary.Perf.Min) }
  if ($t.Exposed) { $lines += ('  thermal: thermal-zone temperature highest {0:n1} C, median {1:n1} C at the end' -f $t.Max, $t.LastMedian) }
  if ($Summary.GpuTemp.Exposed) { $lines += ('  thermal: graphics card temperature highest {0:n0} C' -f $Summary.GpuTemp.Max) }
  if ($null -ne $Summary.LimitBelow100) { $lines += ('  thermal: performance limit below 100% in {0} of {1} samples' -f $Summary.LimitBelow100, $Summary.LimitSamples) }
  if ($null -ne $Summary.PassiveBelow100) { $lines += ('  thermal: passive cooling limit below 100% in {0} of {1} samples' -f $Summary.PassiveBelow100, $Summary.PassiveSamples) }
  if ($Event37 -ge 0) { $lines += ('  thermal: Windows event 37 (Kernel-Processor-Power, the processor ran below its maximum): {0} during the run' -f $Event37) }
  $lines += ('  power: {0}' -f $(if ($Summary.MainsKnown -eq 0) { 'mains or battery not reported' } elseif ($Summary.AcLost) { ('ON BATTERY in {0} of {1} samples' -f $Summary.AcLostSamples, $Summary.MainsKnown) } else { 'on mains power in every sample' }))
  foreach ($n in @($Summary.Notes)) { $lines += "  thermal note: $n" }
  $lines
}

function Get-PackEvent37Count {
  param([datetime]$Since)
  try {
    $e = @(Get-WinEvent -FilterHashtable @{ LogName = 'System'; ProviderName = 'Microsoft-Windows-Kernel-Processor-Power'; Id = 37; StartTime = $Since } -ErrorAction Stop)
    return $e.Count
  } catch {
    if ($_.Exception.Message -match 'No events were found') { return 0 }
    -1
  }
}

# Starts the sampler. Returns a handle for Stop-PackThermal.
function Start-PackThermal {
  [CmdletBinding()]
  param([Parameter(Mandatory = $true)] [string]$OutCsv, [int]$IntervalSec = 5)
  $stop = $OutCsv + '.stop'
  if (Test-Path -LiteralPath $stop) { Remove-Item -LiteralPath $stop -Force }
  $ps = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
  $script = Join-Path $PSScriptRoot 'thermal-sampler.ps1'
  $p = Start-Process -FilePath $ps -PassThru -WindowStyle Hidden -ArgumentList @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', ('"{0}"' -f $script), '-OutCsv', ('"{0}"' -f $OutCsv), '-StopFile', ('"{0}"' -f $stop), '-IntervalSec', "$IntervalSec")
  $null = $p.Handle
  [pscustomobject]@{ Process = $p; Csv = $OutCsv; Stop = $stop; Started = (Get-Date); IntervalSec = $IntervalSec }
}

function Stop-PackThermal {
  [CmdletBinding()]
  param([Parameter(Mandatory = $true)] $Handle)
  Set-Content -LiteralPath $Handle.Stop -Value 'stop' -Encoding ascii
  if (-not $Handle.Process.WaitForExit(30000)) { try { $Handle.Process.Kill() } catch { } }
  $rows = @()
  if (Test-Path -LiteralPath $Handle.Csv) { $rows = @(ConvertTo-PackThermalRows -Csv $Handle.Csv) }
  $sum = Get-PackThermalSummary -Rows $rows -IntervalSec $Handle.IntervalSec
  $ev = Get-PackEvent37Count -Since $Handle.Started
  [pscustomobject]@{ Summary = $sum; Event37 = $ev; Lines = @(Get-PackThermalLines -Summary $sum -Event37 $ev); Csv = $Handle.Csv }
}

Export-ModuleMember -Function Get-PackMedian, ConvertTo-PackThermalRows, Get-PackThermalSummary, Get-PackThermalLines, Get-PackEvent37Count, Start-PackThermal, Stop-PackThermal
