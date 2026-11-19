# The quiet-machine rule and the driver guard shared by the gate scripts (owner, 17 and 19 Nov 2026).
Set-StrictMode -Version Latest

# The Intel and NVIDIA display drivers as Windows reports them: 'name=version' text, one entry per adapter.
function Get-WbGpuDrivers {
  $rows = @(Get-CimInstance Win32_VideoController -ErrorAction SilentlyContinue | Where-Object { $_.Name -match 'Intel|NVIDIA' } | Sort-Object Name | ForEach-Object { "$($_.Name)=$($_.DriverVersion)" })
  ($rows -join '; ')
}

# Quiet check with the gate lock held: host CPU under $CpuMax percent averaged over $Seconds, no other heavy process
# (pwsh, cargo, robocopy, python, wb-* and the like) busy, and the owner's programs not running.
#   Steam (steam, steamwebhelper, steamservice) and the wallpaper renderer (wallpaper32, wallpaper64, webwallpaper32)
#   block the check; when one is running the check waits for the owner to close it, for at most $OwnerWaitMin minutes,
#   then the run is refused. They are never stopped from here.
#   wallpaperservice32 (Wallpaper Engine's background service, near 0% CPU) alone does not block: it is logged.
#   Other reasons: after $FallbackAfterMin minutes of failed checks the run goes on when the host CPU is under
#   $FallbackCpuMax percent and nothing named is busy (verdict 'fallback'); after $GiveUpMin minutes it is refused.
# Every attempt, with the top processes by CPU, goes to $Out (idle-check.json).
function Wait-WbQuiet {
  param([Parameter(Mandatory = $true)] [string]$Out, [double]$CpuMax = 10, [double]$FallbackCpuMax = 15, [int]$FallbackAfterMin = 15, [int]$GiveUpMin = 60, [int]$OwnerWaitMin = 30, [int]$Seconds = 60)
  $blockRx = '^(steam|steamwebhelper|steamservice|wallpaper32|wallpaper64|webwallpaper32)$'
  $loggedRx = '^(wallpaperservice32)$'
  $busyRx = '^(pwsh|powershell|cargo|rustc|robocopy|python|git|link|cl|msbuild|cmake|ninja|node|wb-.*|stelmar.*)$'
  $parentId = 0
  try { $parentId = [int](Get-CimInstance Win32_Process -Filter "ProcessId = $PID").ParentProcessId } catch { $parentId = 0 }
  $counter = New-Object System.Diagnostics.PerformanceCounter('Processor', '% Processor Time', '_Total')
  $null = $counter.NextValue()
  $began = Get-Date
  $ownerSince = $null
  $attempts = @()
  while ($true) {
    $before = @{}
    foreach ($p in @(Get-Process -ErrorAction SilentlyContinue)) {
      $t = $null
      try { $t = $p.TotalProcessorTime.TotalSeconds } catch { $t = $null }
      if ($null -ne $t) { $before[$p.Id] = $t }
    }
    $samples = New-Object System.Collections.Generic.List[double]
    for ($i = 0; $i -lt $Seconds; $i++) { Start-Sleep -Seconds 1; $samples.Add([double]$counter.NextValue()) }
    $busy = @()
    $top = @()
    foreach ($p in @(Get-Process -ErrorAction SilentlyContinue)) {
      if ($p.Id -eq $PID -or $p.Id -eq $parentId -or -not $before.ContainsKey($p.Id)) { continue }
      $t = $null
      try { $t = $p.TotalProcessorTime.TotalSeconds } catch { $t = $null }
      if ($null -eq $t) { continue }
      $d = $t - $before[$p.Id]
      if ($d -ge 0.5) { $top += [pscustomobject]@{ name = $p.ProcessName; cpu_s = [math]::Round($d, 1) } }
      if ($d -ge 3 -and $p.ProcessName -match $busyRx) { $busy += ('{0} {1:F1} CPU-s' -f $p.ProcessName, $d) }
    }
    $avg = ($samples | Measure-Object -Average).Average
    $max = ($samples | Measure-Object -Maximum).Maximum
    $elapsed = ((Get-Date) - $began).TotalMinutes
    $all = @(Get-Process -ErrorAction SilentlyContinue)
    $owner = @($all | Where-Object { $_.ProcessName -match $blockRx } | ForEach-Object { $_.ProcessName } | Sort-Object -Unique)
    $logged = @($all | Where-Object { $_.ProcessName -match $loggedRx } | ForEach-Object { $_.ProcessName } | Sort-Object -Unique)
    if ($owner.Count -gt 0) { if (-not $ownerSince) { $ownerSince = Get-Date } } else { $ownerSince = $null }
    $ownerWaited = $(if ($ownerSince) { ((Get-Date) - $ownerSince).TotalMinutes } else { 0.0 })
    $ok = ($avg -lt $CpuMax -and $busy.Count -eq 0 -and $owner.Count -eq 0)
    $fallback = ((-not $ok) -and $elapsed -ge $FallbackAfterMin -and $avg -lt $FallbackCpuMax -and $busy.Count -eq 0 -and $owner.Count -eq 0)
    $free = [math]::Round((Get-CimInstance Win32_OperatingSystem).FreePhysicalMemory / 1MB, 1)
    $attempts += [pscustomobject]@{
      at = (Get-Date).ToString('o'); cpu_avg = [math]::Round($avg, 1); cpu_max = [math]::Round($max, 0); busy = @($busy)
      owner_programs_blocking = @($owner); logged_not_blocking = @($logged)
      top = @($top | Sort-Object cpu_s -Descending | Select-Object -First 8); free_ram_gb = $free
    }
    $state = $(if ($ok) { 'IDLE' } elseif ($fallback) { 'NOT IDLE, going on (under the fallback limit after the wait)' } else { 'NOT IDLE' })
    Write-Host ("{0} quiet check {1}: host CPU {2:F1}% (max {3:F0}%), busy: {4}, blocking programs: {5}, logged only: {6}, free RAM {7} GB -> {8}" -f (Get-Date -Format 'HH:mm:ss'), $attempts.Count, $avg, $max, $(if ($busy.Count) { $busy -join '; ' } else { 'none' }), $(if ($owner.Count) { $owner -join ', ' } else { 'none' }), $(if ($logged.Count) { $logged -join ', ' } else { 'none' }), $free, $state)
    $verdict = $(if ($ok) { 'idle' } elseif ($fallback) { 'fallback' } elseif ($ownerWaited -ge $OwnerWaitMin -or $elapsed -ge $GiveUpMin) { 'refused' } else { 'waiting' })
    $json = (@{
        verdict = $verdict; cpu_max_limit = $CpuMax; fallback_cpu_limit = $FallbackCpuMax; fallback_after_min = $FallbackAfterMin
        owner_wait_limit_min = $OwnerWaitMin; give_up_min = $GiveUpMin
        seconds = $Seconds; waited_min = [math]::Round($elapsed, 1); attempts = @($attempts)
      } | ConvertTo-Json -Depth 6)
    [IO.File]::WriteAllText($Out, $json, (New-Object Text.UTF8Encoding($false)))
    if ($ok -or $fallback) { return $verdict }
    if ($ownerWaited -ge $OwnerWaitMin) { throw "the owner's programs ($($owner -join ', ')) were still running after $OwnerWaitMin minutes: stopped, not waiting longer (see $Out)" }
    if ($elapsed -ge $GiveUpMin) { throw "the machine did not become quiet within $GiveUpMin minutes (see $Out)" }
    Start-Sleep -Seconds 20
  }
}

Export-ModuleMember -Function Get-WbGpuDrivers, Wait-WbQuiet
