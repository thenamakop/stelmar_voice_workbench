<#
.SYNOPSIS
  The per-process sampler of the day simulation, run as a process of its own (day-sim.ps1 starts it next to the app).

.DESCRIPTION
  Every IntervalSeconds one row of CPU and GPU use goes to Path (process-samples.csv). It is a separate process because
  one sample reads the Windows GPU Engine counters, which took about 100 s while a GPU engine was working: inside the
  script's own loop that made the loop run once in 2 minutes, so the script saw the app's start and stop lines up to
  2 minutes late, started the next session's player before the last session's reference capture had stopped and put the
  player 90 s into the recording (day simulation of 20 Nov 2026). Its own cost shows as 'script' in the rows.
  It ends when StopFile appears.
#>
param(
  [Parameter(Mandatory = $true)] [string]$Path,
  [Parameter(Mandatory = $true)] [int]$RootId,
  [Parameter(Mandatory = $true)] [string]$StopFile,
  [int]$IntervalSeconds = 10
)
$ErrorActionPreference = 'Continue'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
Import-Module (Join-Path $here '..\lib\AppProcess.psm1') -Force

# Per-process samples (owner, 18 Nov 2026): every 10 s one row of CPU (percent of the whole host) and GPU (percent
# of an engine, 3D and compute engines of each process summed) by who used it: the batch engines, the app, its
# WebView2 processes, the test player, this script (the sampler's own cost) and everything else (host total less
# the named ones). It tells how much of a unit's slowdown against a quiet run is our own app and how much is noise
# outside it. day_sim_report.py reads process-samples.csv for each unit's window.
$script:SampleCpu = @{}
$script:SampleAt = $null
$script:SampleTree = @()
$script:SampleTreeAt = $null
$script:HostCounter = $null

function Add-ProcessSample {
  param([string]$Path, [int]$RootId)
  if (-not $script:HostCounter) {
    $script:HostCounter = New-Object System.Diagnostics.PerformanceCounter('Processor', '% Processor Time', '_Total')
    $null = $script:HostCounter.NextValue()
  }
  $now = Get-Date
  if (-not $script:SampleTreeAt -or ($now - $script:SampleTreeAt).TotalSeconds -ge 60) {
    $script:SampleTree = @(Get-WbProcessTree $RootId | ForEach-Object { [int]$_.ProcessId })
    $script:SampleTreeAt = $now
  }
  $procs = @(Get-Process -ErrorAction SilentlyContinue)
  $cat = @{}
  $cpu = @{ engine = 0.0; app = 0.0; webview = 0.0; player = 0.0; script = 0.0 }
  $dt = if ($script:SampleAt) { ($now - $script:SampleAt).TotalSeconds } else { 0 }
  $cores = [Environment]::ProcessorCount
  $next = @{}
  foreach ($p in $procs) {
    $name = $p.ProcessName
    $c = $null
    if ($p.Id -eq $PID) { $c = 'script' }
    elseif ($name -like 'wb-engine*') { $c = 'engine' }
    elseif ($name -eq 'stelmar-voice-workbench') { $c = 'app' }
    elseif ($name -eq 'msedgewebview2' -and ($script:SampleTree -contains $p.Id)) { $c = 'webview' }
    elseif ($name -eq 'wb-testbench') { $c = 'player' }
    if (-not $c) { continue }
    $cat[$p.Id] = $c
    $t = try { $p.TotalProcessorTime.TotalSeconds } catch { $null }
    if ($null -eq $t) { continue }
    $next[$p.Id] = $t
    if ($dt -gt 0 -and $script:SampleCpu.ContainsKey($p.Id)) {
      $cpu[$c] += [math]::Max(0.0, $t - $script:SampleCpu[$p.Id]) / $dt / $cores * 100.0
    }
  }
  $script:SampleCpu = $next
  $script:SampleAt = $now
  $hostCpu = [double]$script:HostCounter.NextValue()
  $named = $cpu.engine + $cpu.app + $cpu.webview + $cpu.player + $cpu.script
  $other = [math]::Max(0.0, $hostCpu - $named)
  # GPU: sum of the 3D and compute engines of each process, by category; the rest of the card is 'other'.
  $gpu = @{ engine = 0.0; app = 0.0; webview = 0.0; other = 0.0 }
  $byLuid = @{}
  try {
    $samples = (Get-Counter '\GPU Engine(*)\Utilization Percentage' -ErrorAction Stop).CounterSamples
    foreach ($g in $samples) {
      if ($g.InstanceName -notmatch '^pid_(\d+)_.*engtype_(3D|Compute)') { continue }
      $gid = [int]$Matches[1]
      $k = if ($cat.ContainsKey($gid) -and $gpu.ContainsKey($cat[$gid])) { $cat[$gid] } else { 'other' }
      $gpu[$k] += [double]$g.CookedValue
      if ($g.InstanceName -match 'luid_0x([0-9a-fA-F]+)_0x([0-9a-fA-F]+)') {
        $lk = "$($Matches[1]).$($Matches[2])"
        if (-not $byLuid.ContainsKey($lk)) { $byLuid[$lk] = @{ engine = 0.0; app = 0.0; webview = 0.0; other = 0.0 } }
        $byLuid[$lk][$k] += [double]$g.CookedValue
      }
    }
  } catch { }
  if ($dt -gt 0) {
    if (-not (Test-Path -LiteralPath $Path)) {
      Set-Content -LiteralPath $Path -Encoding ascii -Value 'unix_s,host_cpu,engine_cpu,app_cpu,webview_cpu,player_cpu,script_cpu,other_cpu,engine_gpu,app_gpu,webview_gpu,other_gpu,gpu_by_adapter'
    }
    # Per adapter (luid): engine/app/webview/other in percent, adapters separated by ';'.
    $ad = ($byLuid.GetEnumerator() | Sort-Object Name | ForEach-Object { '{0}={1:F0}/{2:F0}/{3:F0}/{4:F0}' -f $_.Name, $_.Value.engine, $_.Value.app, $_.Value.webview, $_.Value.other }) -join ';'
    $row = '{0},{1:F1},{2:F1},{3:F1},{4:F1},{5:F1},{6:F1},{7:F1},{8:F1},{9:F1},{10:F1},{11:F1},{12}' -f [DateTimeOffset]::UtcNow.ToUnixTimeSeconds(), $hostCpu, $cpu.engine, $cpu.app, $cpu.webview, $cpu.player, $cpu.script, $other, $gpu.engine, $gpu.app, $gpu.webview, $gpu.other, $ad
    Add-Content -LiteralPath $Path -Encoding ascii -Value $row
  }
}

while (-not (Test-Path -LiteralPath $StopFile)) {
  $t0 = Get-Date
  try { Add-ProcessSample -Path $Path -RootId $RootId } catch { }
  $left = $IntervalSeconds - ((Get-Date) - $t0).TotalSeconds
  if ($left -gt 0) { Start-Sleep -Milliseconds ([int]($left * 1000)) }
}
