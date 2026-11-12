# Per-second sampling of the machine during a gate run (diagnostic runs), and the machine state a
# gate starts in. Nothing here is gated; it is evidence for finding the cause of a failure.
#
# The sampler is typeperf.exe (part of Windows) reading Windows performance counters once a second
# into a CSV. It is pinned to logical processors OUTSIDE the job's cores and the rig's cores, and
# runs at High priority class so it keeps sampling under the load test. ASCII only; Windows
# PowerShell 5.1.
$ErrorActionPreference = 'Stop'

# Counters, one per line. The job's logical processors are 16-19 on the Legion (emu-low), so those
# four are read one by one; the others are read as a whole.
function Get-WbSamplerCounters {
  [CmdletBinding()]
  param([int[]]$JobProcessors = @(16, 17, 18, 19))
  $c = New-Object System.Collections.Generic.List[string]
  foreach ($p in $JobProcessors) {
    foreach ($n in '% Processor Time', '% DPC Time', '% Interrupt Time', '% Processor Performance', 'Processor Frequency', '% Idle Time') {
      $c.Add("\Processor Information(0,$p)\$n")
    }
  }
  foreach ($n in '% Processor Time', '% DPC Time', '% Interrupt Time') { $c.Add("\Processor Information(_Total)\$n") }
  $c.Add('\Process(*)\% Processor Time')
  $c.Add('\Process(*)\ID Process')
  $c.Add('\Memory\Available MBytes')
  $c.Add('\Memory\Pages/sec')
  $c.Add('\Memory\Page Faults/sec')
  $c.Add('\PhysicalDisk(*)\Avg. Disk sec/Write')
  $c.Add('\PhysicalDisk(*)\Avg. Disk sec/Read')
  $c.Add('\PhysicalDisk(*)\Disk Bytes/sec')
  $c.Add('\System\Context Switches/sec')
  $c.Add('\System\Processor Queue Length')
  $c.Add('\Network Interface(*)\Bytes Total/sec')
  @($c.ToArray())
}

# Starts the sampler: a typeperf process writing -OutCsv once a second for -Seconds. Returns the
# process. -Mask is the affinity mask it is pinned to (default logical processors 0-11).
function Start-WbSampler {
  [CmdletBinding()]
  param(
    [Parameter(Mandatory = $true)] [string]$OutCsv,
    [Parameter(Mandatory = $true)] [int]$Seconds,
    [int[]]$JobProcessors = @(16, 17, 18, 19),
    [long]$Mask = 0xFFF
  )
  $cf = $OutCsv + '.counters.txt'
  [IO.File]::WriteAllLines($cf, (Get-WbSamplerCounters -JobProcessors $JobProcessors))
  $exe = Join-Path $env:SystemRoot 'System32\typeperf.exe'
  $a = '-cf "{0}" -si 1 -sc {1} -f CSV -o "{2}" -y' -f $cf, $Seconds, $OutCsv
  $p = Start-Process -FilePath $exe -ArgumentList $a -PassThru -WindowStyle Hidden -RedirectStandardOutput ($OutCsv + '.out') -RedirectStandardError ($OutCsv + '.err')
  $null = $p.Handle
  try { $p.PriorityClass = [Diagnostics.ProcessPriorityClass]::High } catch { }
  try { $p.ProcessorAffinity = [IntPtr]$Mask } catch { }
  $p
}

function Stop-WbSampler {
  [CmdletBinding()]
  param($Process)
  if ($null -eq $Process) { return }
  try { if (-not $Process.HasExited) { $Process.Kill(); [void]$Process.WaitForExit(10000) } } catch { }
}

# Reads a typeperf CSV: returns Names (column titles, without the host), and Rows (Time as
# DateTime, Values as double[], NaN where the counter had no value).
function Read-WbSamplerCsv {
  [CmdletBinding()]
  param([Parameter(Mandatory = $true)] [string]$Path)
  $rows = New-Object System.Collections.Generic.List[object]
  $names = @()
  $first = $true
  $inv = [Globalization.CultureInfo]::InvariantCulture
  foreach ($line in [IO.File]::ReadLines($Path)) {
    if (-not $line) { continue }
    # Fields are quoted and comma-separated; counter names contain no quotes and no commas other than
    # the "0,16" of a processor instance, which is inside the quotes.
    $f = @([regex]::Matches($line, '"([^"]*)"') | ForEach-Object { $_.Groups[1].Value })
    if ($f.Count -lt 2) { continue }
    if ($first) { $names = @($f[1..($f.Count - 1)] | ForEach-Object { $_ -replace '^\\\\[^\\]+', '' }); $first = $false; continue }
    $t = [datetime]::MinValue
    if (-not [datetime]::TryParseExact($f[0], 'MM/dd/yyyy HH:mm:ss.fff', $inv, 'None', [ref]$t)) { continue }
    $v = New-Object double[] ($f.Count - 1)
    for ($i = 1; $i -lt $f.Count; $i++) {
      $d = 0.0
      if ([double]::TryParse($f[$i], 'Float', $inv, [ref]$d)) { $v[$i - 1] = $d } else { $v[$i - 1] = [double]::NaN }
    }
    $rows.Add([pscustomobject]@{ Time = $t; Values = $v })
  }
  [pscustomobject]@{ Names = $names; Rows = @($rows.ToArray()) }
}

# The state a gate starts in, as evidence lines: how long the machine has been up (the first hour
# after a boot is busy with start-up work), the power plan, mains power, and the processor power
# settings that decide how E-cores are used.
function Get-WbMachineStateLines {
  [CmdletBinding()]
  param()
  $lines = New-Object System.Collections.Generic.List[string]
  try {
    $boot = (Get-CimInstance Win32_OperatingSystem).LastBootUpTime
    try {
      $ev = Get-WinEvent -FilterHashtable @{ LogName = 'System'; ProviderName = 'Microsoft-Windows-Kernel-General'; Id = 12 } -MaxEvents 1 -ErrorAction Stop
      if ($ev.TimeCreated -gt $boot) { $boot = $ev.TimeCreated }
    } catch { }
    $lines.Add(('machine state at start: up {0:n0} min (last full start of the kernel {1:yyyy-MM-dd HH:mm:ss})' -f ((Get-Date) - $boot).TotalMinutes, $boot))
  } catch { $lines.Add("machine state at start: uptime not readable ($($_.Exception.Message))") }
  try {
    $scheme = (@(& powercfg.exe /getactivescheme) -join ' ') -replace '^Power Scheme GUID:\s*', ''
    $lines.Add("power plan: $scheme")
  } catch { $lines.Add('power plan: not readable') }
  try {
    $bat = @(Get-CimInstance Win32_Battery -ErrorAction Stop)
    $lines.Add($(if ($bat.Count -eq 0) { 'mains power: no battery found' } else { 'mains power: ' + $(if ($bat[0].BatteryStatus -in 2, 6, 7, 8, 9) { 'plugged in' } else { 'ON BATTERY' }) + " (battery $($bat[0].EstimatedChargeRemaining)%)" }))
  } catch { $lines.Add('mains power: not readable') }
  foreach ($s in @(@('PERFEPP', 'energy/performance preference'), @('PROCTHROTTLEMIN', 'minimum processor state %'), @('PROCTHROTTLEMAX', 'maximum processor state %'), @('CPMINCORES', 'minimum unparked cores %'), @('CPMINCORES1', 'minimum unparked cores % (efficiency class 1)'))) {
    try {
      $o = @(& powercfg.exe /q SCHEME_CURRENT SUB_PROCESSOR $s[0])
      $ac = @($o | Where-Object { $_ -match 'Current AC Power Setting Index' } | ForEach-Object { ($_ -replace '.*:\s*', '').Trim() })
      if ($ac.Count -gt 0) { $lines.Add("  $($s[1]) on mains: $($ac -join ', ')") }
    } catch { }
  }
  @($lines.ToArray())
}

Export-ModuleMember -Function Get-WbSamplerCounters, Start-WbSampler, Stop-WbSampler, Read-WbSamplerCsv, Get-WbMachineStateLines
