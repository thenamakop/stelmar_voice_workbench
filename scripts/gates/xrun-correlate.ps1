<#
.SYNOPSIS
  Lines up the app's driver xruns with the per-second machine sampler of the same run.

.DESCRIPTION
  For each driver xrun in the app's session.log (and the device-time jump the position check saw at
  the same moment, which is how long the capture thread was away), looks at the sampler rows from 2 s
  before the start of that stall to 1 s after the xrun: the job's logical processors one by one (busy,
  DPC, interrupt, performance %, frequency), the whole machine's DPC and interrupt time, the busiest
  processes, disk write and read latency, free memory, page traffic, context switches, network
  traffic. Then the same figures for the seconds that are not near any xrun, so a reader can see what
  is different at the xruns. Also lists the System-log events during the run (Kernel-Processor-Power
  event 37 is "speed limited by firmware": thermal or power throttling).

  The sampler is typeperf, once a second, so a stall shorter than a second is averaged into one
  sample: this finds what is busy around an xrun, not which instruction was running. The evidence is
  written by this script only (scripts/lib/Redact.psm1 removes personal data).

    powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\gates\xrun-correlate.ps1 -RunDir D:\STELMAR-Gate\load-emu-low-... -Out docs\evidence\...\load-emu-low.xrun-correlation.txt
#>
[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)] [string]$RunDir,
  [string]$SessionDir = '',
  [string]$SamplerCsv = '',
  [Parameter(Mandatory = $true)] [string]$Out,
  [string]$Machine = 'Legion',
  [int[]]$JobProcessors = @(16, 17, 18, 19)
)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo = (Resolve-Path (Join-Path $here '..\..')).Path
Import-Module (Join-Path $repo 'scripts\lib\PsEnv.psm1') -Force
Set-WbCleanPSModulePath
Set-WbInvariantCulture
Import-Module (Join-Path $repo 'scripts\lib\GateRun.psm1') -Force
Import-Module (Join-Path $repo 'scripts\lib\Redact.psm1') -Force
Import-Module (Join-Path $repo 'scripts\lib\Sampler.psm1') -Force

if (-not $SamplerCsv) { $SamplerCsv = Join-Path $RunDir 'sampler.csv' }
if (-not $SessionDir) { $SessionDir = (Get-ChildItem (Join-Path $RunDir 'rec') -Directory | ForEach-Object { Get-ChildItem $_.FullName -Directory } | Select-Object -First 1).FullName }
$log = Join-Path $SessionDir 'session.log'
$inv = [Globalization.CultureInfo]::InvariantCulture

# ---- the xruns and how long the capture thread was away ----
$xruns = New-Object System.Collections.Generic.List[object]
$breaks = New-Object System.Collections.Generic.List[object]
foreach ($l in [IO.File]::ReadLines($log)) {
  if ($l -match '^(\S+) gap cause=xrun n=(\d+) .*audio_ms=(\d+)') {
    $xruns.Add([pscustomobject]@{ N = [int]$Matches[2]; At = [DateTimeOffset]::Parse($Matches[1], $inv).LocalDateTime; AudioMs = [double]$Matches[3]; StallMs = 0.0 })
  } elseif ($l -match '^(\S+) position_break .*dev_ns=(-?\d+)') {
    $breaks.Add([pscustomobject]@{ At = [DateTimeOffset]::Parse($Matches[1], $inv).LocalDateTime; DevMs = [double]$Matches[2] / 1e6 })
  }
}
foreach ($x in $xruns) {
  $b = $breaks | Where-Object { $_.At -ge $x.At -and ($_.At - $x.At).TotalMilliseconds -le 500 } | Select-Object -First 1
  if ($b) { $x.StallMs = [math]::Max(0.0, $b.DevMs) }
}

# ---- the sampler ----
$s = Read-WbSamplerCsv -Path $SamplerCsv
$names = $s.Names
$rows = @($s.Rows)
if ($rows.Count -eq 0) { throw "the sampler file has no rows: $SamplerCsv" }
function Col([string]$Name) { [array]::IndexOf($names, $Name) }
function Get-Val($Row, [int]$i) { if ($i -ge 0) { $Row.Values[$i] } else { [double]::NaN } }
$coreCols = @{}
foreach ($p in $JobProcessors) {
  $coreCols[$p] = [pscustomobject]@{
    Busy = Col "\Processor Information(0,$p)\% Processor Time"; Dpc = Col "\Processor Information(0,$p)\% DPC Time"
    Int = Col "\Processor Information(0,$p)\% Interrupt Time"; Perf = Col "\Processor Information(0,$p)\% Processor Performance"
    Freq = Col "\Processor Information(0,$p)\Processor Frequency"
  }
}
$totDpc = Col '\Processor Information(_Total)\% DPC Time'
$totInt = Col '\Processor Information(_Total)\% Interrupt Time'
$totBusy = Col '\Processor Information(_Total)\% Processor Time'
$availMb = Col '\Memory\Available MBytes'
$pagesSec = Col '\Memory\Pages/sec'
$faultsSec = Col '\Memory\Page Faults/sec'
$ctxSw = Col '\System\Context Switches/sec'
$queue = Col '\System\Processor Queue Length'
$diskW = @(for ($i = 0; $i -lt $names.Count; $i++) { if ($names[$i] -like '\PhysicalDisk(*)\Avg. Disk sec/Write' -and $names[$i] -notlike '*(_Total)*') { $i } })
$diskR = @(for ($i = 0; $i -lt $names.Count; $i++) { if ($names[$i] -like '\PhysicalDisk(*)\Avg. Disk sec/Read' -and $names[$i] -notlike '*(_Total)*') { $i } })
$netB = @(for ($i = 0; $i -lt $names.Count; $i++) { if ($names[$i] -like '\Network Interface(*)\Bytes Total/sec') { $i } })
$procCols = @(for ($i = 0; $i -lt $names.Count; $i++) {
    if ($names[$i] -match '^\\Process\((.+)\)\\% Processor Time$' -and $Matches[1] -ne '_Total' -and $Matches[1] -ne 'Idle') {
      $inst = $Matches[1]
      [pscustomobject]@{ Inst = $inst; Cpu = $i; Id = (Col "\Process($inst)\ID Process") }
    }
  })

function Max-Of($Row, [int[]]$Cols) { $m = [double]::NaN; foreach ($c in $Cols) { $v = $Row.Values[$c]; if (-not [double]::IsNaN($v) -and ([double]::IsNaN($m) -or $v -gt $m)) { $m = $v } }; $m }
function Window([datetime]$From, [datetime]$To) { @($rows | Where-Object { $_.Time -ge $From -and $_.Time -le $To }) }
function Fmt([double]$v, [string]$f = 'n1') { if ([double]::IsNaN($v)) { 'n/a' } else { $v.ToString($f, $inv) } }

# Summary of a set of sampler rows.
function Summarize($Set) {
  $o = [ordered]@{}
  foreach ($p in $JobProcessors) {
    $c = $coreCols[$p]
    $o["cpu$p busy max %"] = ($Set | ForEach-Object { Get-Val $_ $c.Busy } | Measure-Object -Maximum).Maximum
    $o["cpu$p DPC max %"] = ($Set | ForEach-Object { Get-Val $_ $c.Dpc } | Measure-Object -Maximum).Maximum
    $o["cpu$p interrupt max %"] = ($Set | ForEach-Object { Get-Val $_ $c.Int } | Measure-Object -Maximum).Maximum
    $o["cpu$p performance min %"] = ($Set | ForEach-Object { Get-Val $_ $c.Perf } | Measure-Object -Minimum).Minimum
    $o["cpu$p frequency min MHz"] = ($Set | ForEach-Object { Get-Val $_ $c.Freq } | Measure-Object -Minimum).Minimum
  }
  $o['machine DPC max %'] = ($Set | ForEach-Object { Get-Val $_ $totDpc } | Measure-Object -Maximum).Maximum
  $o['machine interrupt max %'] = ($Set | ForEach-Object { Get-Val $_ $totInt } | Measure-Object -Maximum).Maximum
  $o['machine busy max %'] = ($Set | ForEach-Object { Get-Val $_ $totBusy } | Measure-Object -Maximum).Maximum
  $o['disk write latency max ms'] = 1000.0 * (($Set | ForEach-Object { Max-Of $_ $diskW } | Measure-Object -Maximum).Maximum)
  $o['disk read latency max ms'] = 1000.0 * (($Set | ForEach-Object { Max-Of $_ $diskR } | Measure-Object -Maximum).Maximum)
  $o['free memory min MB'] = ($Set | ForEach-Object { Get-Val $_ $availMb } | Measure-Object -Minimum).Minimum
  $o['pages/s max'] = ($Set | ForEach-Object { Get-Val $_ $pagesSec } | Measure-Object -Maximum).Maximum
  $o['page faults/s max'] = ($Set | ForEach-Object { Get-Val $_ $faultsSec } | Measure-Object -Maximum).Maximum
  $o['context switches/s max'] = ($Set | ForEach-Object { Get-Val $_ $ctxSw } | Measure-Object -Maximum).Maximum
  $o['processor queue max'] = ($Set | ForEach-Object { Get-Val $_ $queue } | Measure-Object -Maximum).Maximum
  $o['network KB/s max'] = ($Set | ForEach-Object { Max-Of $_ $netB } | Measure-Object -Maximum).Maximum / 1024.0
  $o
}
function Top-Processes($Set, [int]$N = 5) {
  $best = @{}
  foreach ($r in $Set) {
    foreach ($pc in $procCols) {
      $v = $r.Values[$pc.Cpu]
      if ([double]::IsNaN($v)) { continue }
      if (-not $best.ContainsKey($pc.Inst) -or $v -gt $best[$pc.Inst].Cpu) { $best[$pc.Inst] = [pscustomobject]@{ Inst = $pc.Inst; Cpu = $v; Pid = $(if ($pc.Id -ge 0) { $r.Values[$pc.Id] } else { [double]::NaN }) } }
    }
  }
  @($best.Values | Sort-Object Cpu -Descending | Select-Object -First $N)
}

$lines = New-Object System.Collections.Generic.List[string]
$lines.Add("source: scripts/gates/xrun-correlate.ps1 (written by the script; nothing in this file is typed by hand)")
$lines.Add("machine: $Machine")
$lines.Add("run folder: $RunDir (not in the repository)")
$lines.Add("sampler: $($rows.Count) rows, $($rows[0].Time.ToString('HH:mm:ss', $inv)) to $($rows[-1].Time.ToString('HH:mm:ss', $inv)), once a second (typeperf, pinned to logical processors 0-11); $($names.Count) counters; the job's logical processors: $($JobProcessors -join ', ')")
$lines.Add("driver xruns in the app's session.log: $($xruns.Count)")
$lines.Add('limits: a sample is a one-second average, so a stall of 30 to 500 ms is averaged into one or two samples; per-process figures are percent of ONE core and not tied to a core; the evidence shows what is busy around an xrun, not which thread ran.')
$lines.Add('')

# which seconds are "near" an xrun
$near = New-Object System.Collections.Generic.HashSet[int]
$k = 0
foreach ($r in $rows) {
  foreach ($x in $xruns) {
    $from = $x.At.AddMilliseconds(-$x.StallMs - 2000); $to = $x.At.AddSeconds(1)
    if ($r.Time -ge $from -and $r.Time -le $to) { [void]$near.Add($k); break }
  }
  $k++
}
$nearRows = @(); $farRows = @(); $k = 0
foreach ($r in $rows) { if ($near.Contains($k)) { $nearRows += $r } else { $farRows += $r }; $k++ }
$lines.Add("seconds near an xrun (from 2 s before the stall to 1 s after the xrun): $($nearRows.Count); other seconds: $($farRows.Count)")
$lines.Add('')

$allSum = Summarize $farRows
$lines.Add('== per xrun ==')
foreach ($x in $xruns) {
  $from = $x.At.AddMilliseconds(-$x.StallMs - 2000); $to = $x.At.AddSeconds(1)
  $w = Window $from $to
  $lines.Add(('xrun #{0} at {1} (audio {2:n1} s); the capture thread was away about {3:n0} ms; sampler rows in the window: {4}' -f $x.N, $x.At.ToString('HH:mm:ss.fff', $inv), ($x.AudioMs / 1000.0), $x.StallMs, $w.Count))
  if ($w.Count -eq 0) { $lines.Add('    no sampler row in the window'); continue }
  $sum = Summarize $w
  $parts = @()
  foreach ($p in $JobProcessors) { $parts += ('cpu{0}: busy {1}%, DPC {2}%, int {3}%, perf {4}%, {5} MHz' -f $p, (Fmt $sum["cpu$p busy max %"] 'n0'), (Fmt $sum["cpu$p DPC max %"]), (Fmt $sum["cpu$p interrupt max %"]), (Fmt $sum["cpu$p performance min %"] 'n0'), (Fmt $sum["cpu$p frequency min MHz"] 'n0')) }
  $lines.Add('    job cores (worst second in the window): ' + ($parts -join ' | '))
  $lines.Add(('    machine: DPC max {0}%, interrupt max {1}%, busy max {2}%, processor queue max {3}; disk write latency max {4} ms, read {5} ms; free memory min {6} MB; pages/s max {7}; context switches/s max {8}; network max {9} KB/s' -f (Fmt $sum['machine DPC max %']), (Fmt $sum['machine interrupt max %']), (Fmt $sum['machine busy max %'] 'n0'), (Fmt $sum['processor queue max'] 'n0'), (Fmt $sum['disk write latency max ms']), (Fmt $sum['disk read latency max ms']), (Fmt $sum['free memory min MB'] 'n0'), (Fmt $sum['pages/s max'] 'n0'), (Fmt $sum['context switches/s max'] 'n0'), (Fmt $sum['network KB/s max'] 'n0')))
  $top = Top-Processes $w 5
  $lines.Add('    busiest processes in the window (percent of one core, worst second): ' + (($top | ForEach-Object { '{0} (pid {1}) {2}%' -f $_.Inst, (Fmt $_.Pid 'n0'), (Fmt $_.Cpu 'n0') }) -join '; '))
}
$lines.Add('')
$lines.Add('== all the other seconds of the run, for comparison (worst second over all of them) ==')
$parts = @()
foreach ($p in $JobProcessors) { $parts += ('cpu{0}: busy {1}%, DPC {2}%, int {3}%, perf {4}%, {5} MHz' -f $p, (Fmt $allSum["cpu$p busy max %"] 'n0'), (Fmt $allSum["cpu$p DPC max %"]), (Fmt $allSum["cpu$p interrupt max %"]), (Fmt $allSum["cpu$p performance min %"] 'n0'), (Fmt $allSum["cpu$p frequency min MHz"] 'n0')) }
$lines.Add('    job cores: ' + ($parts -join ' | '))
$lines.Add(('    machine: DPC max {0}%, interrupt max {1}%, busy max {2}%, processor queue max {3}; disk write latency max {4} ms, read {5} ms; free memory min {6} MB; pages/s max {7}; context switches/s max {8}; network max {9} KB/s' -f (Fmt $allSum['machine DPC max %']), (Fmt $allSum['machine interrupt max %']), (Fmt $allSum['machine busy max %'] 'n0'), (Fmt $allSum['processor queue max'] 'n0'), (Fmt $allSum['disk write latency max ms']), (Fmt $allSum['disk read latency max ms']), (Fmt $allSum['free memory min MB'] 'n0'), (Fmt $allSum['pages/s max'] 'n0'), (Fmt $allSum['context switches/s max'] 'n0'), (Fmt $allSum['network KB/s max'] 'n0')))
$lines.Add('    busiest processes over those seconds: ' + ((Top-Processes $farRows 8 | ForEach-Object { '{0} (pid {1}) {2}%' -f $_.Inst, (Fmt $_.Pid 'n0'), (Fmt $_.Cpu 'n0') }) -join '; '))
$lines.Add('')
$lines.Add('== processes that were busy near an xrun and (nearly) idle otherwise ==')
$nearTop = Top-Processes $nearRows 40
$farTop = @{}; foreach ($t in (Top-Processes $farRows 400)) { $farTop[$t.Inst] = $t.Cpu }
$odd = @($nearTop | Where-Object { $_.Cpu -ge 5 -and (-not $farTop.ContainsKey($_.Inst) -or $farTop[$_.Inst] -lt 0.25 * $_.Cpu) })
if ($odd.Count -eq 0) { $lines.Add('    none: every process that was busy near an xrun was about as busy at other times') }
foreach ($o in $odd) { $lines.Add(('    {0} (pid {1}): up to {2}% near an xrun, up to {3}% in the other seconds' -f $o.Inst, (Fmt $o.Pid 'n0'), (Fmt $o.Cpu 'n0'), $(if ($farTop.ContainsKey($o.Inst)) { Fmt $farTop[$o.Inst] 'n0' } else { 'not seen' }))) }

# ---- the Windows event log during the run ----
$lines.Add('')
$lines.Add('== System log during the sampler (Kernel-Processor-Power event 37 = speed limited by firmware) ==')
$s0 = $rows[0].Time.AddSeconds(-5); $s1 = $rows[-1].Time.AddSeconds(5)
try {
  $ev = @(Get-WinEvent -FilterHashtable @{ LogName = 'System'; StartTime = $s0; EndTime = $s1 } -ErrorAction Stop | Sort-Object TimeCreated)
  $lines.Add("System events between $($s0.ToString('HH:mm:ss', $inv)) and $($s1.ToString('HH:mm:ss', $inv)): $($ev.Count)")
  foreach ($e in ($ev | Select-Object -First 60)) {
    $m = ($e.Message -replace '\s+', ' '); if ($m.Length -gt 160) { $m = $m.Substring(0, 160) }
    $lines.Add(('    {0} id={1} {2}: {3}' -f $e.TimeCreated.ToString('HH:mm:ss', $inv), $e.Id, $e.ProviderName, $m))
  }
  if ($ev.Count -gt 60) { $lines.Add("    ... $($ev.Count - 60) more") }
} catch { $lines.Add("System events between $($s0.ToString('HH:mm:ss', $inv)) and $($s1.ToString('HH:mm:ss', $inv)): none ($($_.Exception.Message))") }
try {
  $ev37 = @(Get-WinEvent -FilterHashtable @{ LogName = 'System'; ProviderName = 'Microsoft-Windows-Kernel-Processor-Power'; Id = 37; StartTime = $s0; EndTime = $s1 } -ErrorAction Stop)
  $lines.Add("event 37 (speed limited by firmware) during the sampler: $($ev37.Count)")
} catch { $lines.Add('event 37 (speed limited by firmware) during the sampler: 0') }

$dir = Split-Path -Parent $Out
New-Item -ItemType Directory -Force -Path $dir | Out-Null
Write-WbEvidence -Path $Out -Lines @($lines.ToArray()) -MachineLabel $Machine
Write-Host "xrun correlation written: $Out"
