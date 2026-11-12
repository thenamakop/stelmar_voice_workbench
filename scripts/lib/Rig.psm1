<#
  Rig.psm1: where the test rig runs.

  The player that feeds the virtual cable and the reference capture are part of the test rig, not
  of the emulated laptop. They run outside the emulation job but were not pinned, so they could
  share cores with the job (logical processors 16-19 under emu-low), and a late render callback
  in the player shows up as a break in both captures. They are now pinned to performance cores
  that no emulation profile uses, at HIGH priority class (not realtime). The cores come from
  `wb-emulate rig`, which reads GetLogicalProcessorInformationEx like the emulator does; nothing
  here assumes core numbers. ASCII only; Windows PowerShell 5.1.
#>
Import-Module (Join-Path $PSScriptRoot 'GateRun.psm1')

function ConvertFrom-WbMask {
  param([Parameter(Mandatory = $true)] [string]$Hex)
  [Convert]::ToUInt64(($Hex -replace '^0[xX]', ''), 16)
}

function Format-WbMask {
  param([Parameter(Mandatory = $true)] [uint64]$Mask)
  '0x' + $Mask.ToString('x')
}

# '16-19' for 0xf0000, '12-15' for 0xf000, '0,2' for 0x5.
function Format-WbLogicalList {
  param([Parameter(Mandatory = $true)] [uint64]$Mask)
  $bits = @(0..63 | Where-Object { ($Mask -band ([uint64]1 -shl $_)) -ne 0 })
  $parts = @()
  $i = 0
  while ($i -lt $bits.Count) {
    $j = $i
    while ($j + 1 -lt $bits.Count -and $bits[$j + 1] -eq $bits[$j] + 1) { $j++ }
    $parts += $(if ($j -gt $i) { "$($bits[$i])-$($bits[$j])" } else { "$($bits[$i])" })
    $i = $j + 1
  }
  $parts -join ','
}

# The rig's planned placement for a profile, from wb-emulate. $null for the native profile (no
# job mask to stay out of).
function Get-WbRigPlan {
  param(
    [Parameter(Mandatory = $true)] [string]$Emulate,
    [Parameter(Mandatory = $true)] [string]$Profile
  )
  if ($Profile -eq 'native') { return $null }
  $res = Invoke-WbNative -FilePath $Emulate -ArgumentList @('rig', '--profile', $Profile)
  if ($res.ExitCode -ne 0 -or $res.Output.Count -eq 0) {
    throw "wb-emulate rig --profile $Profile failed (exit $($res.ExitCode)): $((@($res.Error) + @($res.Output)) -join ' ')"
  }
  $j = ($res.Output -join "`n") | ConvertFrom-Json
  [pscustomobject]@{
    Profile       = $Profile
    JobMask       = (ConvertFrom-WbMask $j.job_mask)
    PlayerMask    = (ConvertFrom-WbMask $j.player_mask)
    ReferenceMask = (ConvertFrom-WbMask $j.reference_mask)
    Strict        = [bool]$j.strict
    Note          = $(if ($j.note) { "$($j.note)" } else { $null })
  }
}

# Throws when the player, the reference capture and the job do not have three disjoint, non-empty
# sets of logical processors. Used on the planned masks before anything starts and again on the
# masks read back from the running processes.
function Assert-WbRigPlacement {
  param(
    [Parameter(Mandatory = $true)] [uint64]$JobMask,
    [Parameter(Mandatory = $true)] [uint64]$PlayerMask,
    [Parameter(Mandatory = $true)] [uint64]$ReferenceMask,
    [string]$What = 'planned'
  )
  $problems = @()
  if ($JobMask -eq 0) { $problems += 'the job mask is empty' }
  if ($PlayerMask -eq 0) { $problems += 'the player mask is empty' }
  if ($ReferenceMask -eq 0) { $problems += 'the reference capture mask is empty' }
  $pr = $PlayerMask -band $ReferenceMask
  $pj = $PlayerMask -band $JobMask
  $rj = $ReferenceMask -band $JobMask
  if ($pr -ne 0) { $problems += "the player ($(Format-WbMask $PlayerMask)) and the reference capture ($(Format-WbMask $ReferenceMask)) share logical processors $(Format-WbLogicalList $pr)" }
  if ($pj -ne 0) { $problems += "the player ($(Format-WbMask $PlayerMask)) shares logical processors $(Format-WbLogicalList $pj) with the job ($(Format-WbMask $JobMask))" }
  if ($rj -ne 0) { $problems += "the reference capture ($(Format-WbMask $ReferenceMask)) shares logical processors $(Format-WbLogicalList $rj) with the job ($(Format-WbMask $JobMask))" }
  if ($problems.Count -gt 0) {
    throw "The test rig overlaps the emulated machine ($What): $($problems -join '; '). The gate does not start."
  }
}

# The default CPU sets of a process (Windows CPU sets, set with SetProcessDefaultCpuSets; separate
# from the affinity mask and easy to miss, so the evidence names both). Returns the logical
# processors as 'none' (no set: the process may run anywhere its affinity allows) or a list such as
# '12-13'. CPU set ids are 256 + the logical processor index on a system with one processor group,
# which the Legion is.
function Get-WbProcessCpuSets {
  param([Parameter(Mandatory = $true)] [System.Diagnostics.Process]$Process)
  try {
    if (-not ('WbRig.CpuSets' -as [type])) {
      Add-Type -Namespace WbRig -Name CpuSets -MemberDefinition '[System.Runtime.InteropServices.DllImport("kernel32.dll", SetLastError = true)] public static extern bool GetProcessDefaultCpuSets(System.IntPtr process, uint[] ids, uint count, out uint required);'
    }
    $required = [uint32]0
    $null = [WbRig.CpuSets]::GetProcessDefaultCpuSets($Process.Handle, $null, [uint32]0, [ref]$required)
    if ($required -eq 0) { return 'none (all processors the affinity mask allows)' }
    $ids = New-Object 'uint32[]' $required
    if (-not [WbRig.CpuSets]::GetProcessDefaultCpuSets($Process.Handle, $ids, $required, [ref]$required)) { return 'unreadable (GetProcessDefaultCpuSets failed)' }
    $mask = [uint64]0
    foreach ($id in $ids) { if ($id -ge 256 -and $id -lt 256 + 64) { $mask = $mask -bor ([uint64]1 -shl [int]($id - 256)) } }
    return (Format-WbLogicalList $mask)
  } catch {
    return "unreadable ($($_.Exception.Message))"
  }
}

# Pins a running process to $Mask at HIGH priority class and reads both back, so the evidence
# shows what the process really has (not what was asked for).
function Set-WbRigPlacement {
  param(
    [Parameter(Mandatory = $true)] [System.Diagnostics.Process]$Process,
    [Parameter(Mandatory = $true)] [uint64]$Mask,
    [Parameter(Mandatory = $true)] [string]$Role
  )
  $Process.ProcessorAffinity = [IntPtr][int64]$Mask
  $Process.PriorityClass = [System.Diagnostics.ProcessPriorityClass]::High
  $Process.Refresh()
  [pscustomobject]@{
    Role          = $Role
    ProcessId     = $Process.Id
    RequestedMask = $Mask
    Mask          = [uint64]$Process.ProcessorAffinity.ToInt64()
    Priority      = "$($Process.PriorityClass)"
    CpuSet        = (Get-WbProcessCpuSets -Process $Process)
  }
}

# What the app inside the job really has (read back from the running process).
function Get-WbJobPlacement {
  $app = @(Get-Process stelmar-voice-workbench -ErrorAction SilentlyContinue | Select-Object -First 1)
  if ($app.Count -eq 0) { throw 'The app is not running; cannot read the job''s processor mask' }
  [pscustomobject]@{
    Role          = 'job (the app inside the emulation job)'
    ProcessId     = $app[0].Id
    RequestedMask = [uint64]0
    Mask          = [uint64]$app[0].ProcessorAffinity.ToInt64()
    Priority      = $null
    CpuSet        = (Get-WbProcessCpuSets -Process $app[0])
  }
}

# Pins the player and the reference capture right after they start, reads the placement back, and
# only then creates $GoFile. Both processes wait for it (--go-file) before they open their audio
# streams, so neither runs a single callback unpinned (an earlier version pinned after start and
# the reference capture reported the full mask at stream start). With no plan (native profile)
# nothing is pinned, but the go file is still created. Needs the app only for the job check,
# which Confirm-WbRigAgainstJob does once the app runs.
function Set-WbRigPins {
  param(
    $Plan,
    [Parameter(Mandatory = $true)] [System.Diagnostics.Process]$Player,
    [Parameter(Mandatory = $true)] [System.Diagnostics.Process]$Reference,
    [Parameter(Mandatory = $true)] [string]$GoFile
  )
  $result = $null
  try {
    if ($Plan) {
      $ref = Set-WbRigPlacement -Process $Reference -Mask $Plan.ReferenceMask -Role 'reference capture'
      $play = Set-WbRigPlacement -Process $Player -Mask $Plan.PlayerMask -Role 'player'
      foreach ($p in @($ref, $play)) {
        if ($p.Mask -ne $p.RequestedMask) { throw "$($p.Role) was asked for $(Format-WbMask $p.RequestedMask) and has $(Format-WbMask $p.Mask)." }
        if ($p.Priority -ne 'High') { throw "$($p.Role) has priority class $($p.Priority), not High." }
      }
      $result = [pscustomobject]@{ Plan = $Plan; Job = $null; Player = $play; Reference = $ref }
    }
    Set-Content -LiteralPath $GoFile -Value 'go' -Encoding ascii
  } catch {
    Stop-Process -Id $Player.Id, $Reference.Id -Force -ErrorAction SilentlyContinue
    throw
  }
  $result
}

# With the app running: reads the job's mask back and checks that it is the planned one and that
# neither the player nor the reference capture overlaps it or the other. Stops both processes
# before throwing, so a bad placement ends the gate at its start.
function Confirm-WbRigAgainstJob {
  param(
    $Rig,
    [Parameter(Mandatory = $true)] [System.Diagnostics.Process]$Player,
    [Parameter(Mandatory = $true)] [System.Diagnostics.Process]$Reference
  )
  if ($null -eq $Rig) { return $null }
  try {
    $job = Get-WbJobPlacement
    if ($job.Mask -ne $Rig.Plan.JobMask) {
      throw "The job's mask read back ($(Format-WbMask $job.Mask)) is not the planned one ($(Format-WbMask $Rig.Plan.JobMask))."
    }
    Assert-WbRigPlacement -JobMask $job.Mask -PlayerMask $Rig.Player.Mask -ReferenceMask $Rig.Reference.Mask -What 'read back from the running processes'
  } catch {
    Stop-Process -Id $Player.Id, $Reference.Id -Force -ErrorAction SilentlyContinue
    throw
  }
  $Rig.Job = $job
  $Rig
}

# Evidence lines for the rig placement ($Rig has Plan, Job, Player and Reference, or is $null).
function Get-WbRigLines {
  param($Rig, $Play, $Reference)
  if ($null -eq $Rig) { return @('  rig placement: not pinned (native profile) or not recorded for this run') }
  $lines = @('  rig placement (player and reference capture pinned to performance cores outside the job, HIGH priority class; masks read back from the running processes):')
  foreach ($p in @($Rig.Job, $Rig.Player, $Rig.Reference)) {
    if ($null -eq $p) { continue }
    $own = ''
    if ($p.Role -eq 'player' -and $Play -and (Test-WbHasProperty $Play 'affinity_mask') -and $Play.affinity_mask) { $own = "; the player itself reports $($Play.affinity_mask) at stream start" }
    if ($p.Role -eq 'reference capture' -and $Reference -and (Test-WbHasProperty $Reference 'affinity_mask') -and $Reference.affinity_mask) { $own = "; the reference capture itself reports $($Reference.affinity_mask) at stream start" }
    $prio = $(if ($p.Priority) { ", priority class $($p.Priority)" } else { '' })
    $cpuset = $(if ((Test-WbHasProperty $p "CpuSet") -and $p.CpuSet) { "; CPU set $($p.CpuSet)" } else { "" })
    $lines += "    $($p.Role): pid $($p.ProcessId), mask $(Format-WbMask $p.Mask) (logical processors $(Format-WbLogicalList $p.Mask))$prio$cpuset$own"
  }
  $lines += '    overlap: none (checked on the planned masks before anything started, and on the masks read back from the running processes)'
  if ($Rig.Plan -and $Rig.Plan.Note) { $lines += "    note: $($Rig.Plan.Note)" }
  $lines
}

# Evidence lines for the player's late render callbacks (a gap between render callbacks longer
# than its 100 ms output buffer, the only kind that can run the buffer dry), and how many fall
# within $WindowMs of each given source position (the shared breaks).
function Get-WbLateCallbackLines {
  param($Play, [double[]]$BreakSourceMs = @(), [double]$WindowMs = 500, [string]$Label = 'shared break')
  if ($null -eq $Play -or -not (Test-WbHasProperty $Play 'late_callbacks')) {
    return @('  player late callbacks: not recorded for this run (an earlier player)')
  }
  $events = @($Play.late_callback_events)
  $maxGap = [double]$Play.max_callback_interval_ms
  $gaps = ''
  if (Test-WbHasProperty $Play 'callback_gaps_ge_20ms') {
    $gaps = "; gaps of 20 ms or more: $($Play.callback_gaps_ge_20ms), 30 ms or more: $($Play.callback_gaps_ge_30ms), 50 ms or more: $($Play.callback_gaps_ge_50ms)"
  }
  $lines = @(("  player late callbacks: {0} (a gap between render callbacks longer than the 100 ms output buffer); longest gap between callbacks {1:n1} ms{2}" -f $Play.late_callbacks, $maxGap, $gaps))
  foreach ($e in ($events | Select-Object -First 10)) {
    $lines += ("    late callback at source {0:n3} s: {1:n1} ms after the previous one (output buffer {2:n0} ms)" -f ([double]$e.source_ms / 1000.0), [double]$e.interval_ms, [double]$e.buffer_ms)
  }
  if ($events.Count -gt 10) { $lines += "    ... $($events.Count - 10) more recorded in play.json" }
  foreach ($b in $BreakSourceMs) {
    $near = @($events | Where-Object { [math]::Abs([double]$_.source_ms - $b) -le $WindowMs })
    if ($near.Count -gt 0) {
      $lines += ("    $Label at source {0:n3} s: {1} late player callback(s) within {2:n0} ms of it" -f ($b / 1000.0), $near.Count, $WindowMs)
    } else {
      $lines += ("    $Label at source {0:n3} s: no late player callback within {1:n0} ms of it; the longest callback gap in the whole run was {2:n1} ms, so the player never ran its output buffer dry" -f ($b / 1000.0), $WindowMs, $maxGap)
    }
  }
  $lines
}

# Evidence lines for the reference capture's xruns (each with its callback count, its position in
# the reference recording and its wall-clock time to the millisecond), and the nearest one to each
# shared break. $PlayStarted is when the player's first callback ran (DateTimeOffset): a break at
# source position S happened at about PlayStarted + (S - first source frame heard).
function Get-WbReferenceXrunLines {
  param(
    $RefAnalysis,
    $Reference,
    $PlayStarted,
    [double[]]$BreakSourceMs = @(),
    [string]$Label = 'shared break'
  )
  if ($null -eq $RefAnalysis -or -not (Test-WbHasProperty $RefAnalysis 'content_start_ms')) {
    return @('  reference xruns: not available (no reference analysis)')
  }
  $events = @()
  if (Test-WbHasProperty $RefAnalysis 'xruns') { $events = @($RefAnalysis.xruns) }
  $reported = $(if ($Reference -and (Test-WbHasProperty $Reference 'xruns')) { [int64]$Reference.xruns } else { -1 })
  if ($events.Count -eq 0) {
    return @("  reference xruns: $(if ($reported -ge 0) { $reported } else { 'not recorded' }) (no per-xrun list: an earlier reference capture)")
  }
  $rate = [double]$RefAnalysis.sample_rate
  $first = [double]$RefAnalysis.content_first_source_frame * 1000.0 / $rate
  $lines = @("  reference xruns (WASAPI discontinuity flag, as cpal reports it; logged by the reference capture): $($events.Count)")
  $items = @()
  foreach ($e in $events) {
    $wall = [DateTimeOffset]::Parse($e.wall_clock)
    $src = [double]$e.audio_ms - [double]$RefAnalysis.content_start_ms + $first
    $items += [pscustomobject]@{ N = $e.n; Wall = $wall; SourceMs = $src }
    $where = $(if ($src -lt 0) { 'before the signal starts' } else { ('about source {0:n3} s' -f ($src / 1000.0)) })
    $lines += ("    #{0} at {1:HH:mm:ss.fff} IST: {2} callbacks and {3} frames in; reference recording {4:n3} s; {5}" -f $e.n, $wall, $e.callbacks_before, $e.frames_before, ([double]$e.audio_ms / 1000.0), $where)
  }
  foreach ($b in $BreakSourceMs) {
    if ($null -eq $PlayStarted) { continue }
    $when = ([DateTimeOffset]$PlayStarted).AddMilliseconds($b - $first)
    $near = $items | Sort-Object { [math]::Abs(($_.Wall - $when).TotalMilliseconds) } | Select-Object -First 1
    $text = ("    $Label at source {0:n3} s, {1:HH:mm:ss.fff} IST (to the second: {1:HH:mm:ss}): " -f ($b / 1000.0), $when)
    $d = ($near.Wall - $when).TotalMilliseconds
    if ([math]::Abs($d) -le 2000) {
      if ([math]::Abs($d) -lt 1) {
        $text += ("nearest reference xrun #{0} at {1:HH:mm:ss.fff}, within 1 ms of it" -f $near.N, $near.Wall)
      } else {
        $text += ("nearest reference xrun #{0} at {1:HH:mm:ss.fff}, {2:n0} ms {3}" -f $near.N, $near.Wall, [math]::Abs($d), $(if ($d -lt 0) { 'before it' } else { 'after it' }))
      }
    } else {
      $text += ("no reference xrun within 2 s (nearest: #{0} at {1:HH:mm:ss.fff}, {2:n1} s {3})" -f $near.N, $near.Wall, ([math]::Abs($d) / 1000.0), $(if ($d -lt 0) { 'before' } else { 'after' }))
    }
    $lines += $text
  }
  $lines
}

# Which stress process a wb-testbench command line is: 'job' (the one the inner script starts inside
# the emulation job, with --gpu), 'host' (the load gate's all-processors load outside the job, with
# --threads), or $null for any other command.
function Get-WbStressKind([string]$CommandLine) {
  if (-not $CommandLine -or $CommandLine -notmatch '\sstress(\s|$)') { return $null }
  if ($CommandLine -match '--gpu') { return 'job' }
  if ($CommandLine -match '--threads') { return 'host' }
  return 'other'
}

# The stress processes that run now: priority class, threads with their base priorities, the CPU
# they used over a few seconds (about the number of busy threads) and their affinity. Read from the
# running processes, not from what the script meant to start.
function Get-WbStressInfo {
  param([int]$SampleSeconds = 3)
  $found = @()
  foreach ($p in @(Get-CimInstance -ClassName Win32_Process -Filter "Name='wb-testbench.exe'" -ErrorAction SilentlyContinue)) {
    $kind = Get-WbStressKind $p.CommandLine
    if ($kind) { $found += [pscustomobject]@{ Kind = $kind; Id = [int]$p.ProcessId } }
  }
  $before = @{}
  foreach ($f in $found) {
    try { $before[$f.Id] = (Get-Process -Id $f.Id -ErrorAction Stop).TotalProcessorTime.TotalSeconds } catch { }
  }
  Start-Sleep -Seconds $SampleSeconds
  foreach ($f in $found) {
    try {
      $gp = Get-Process -Id $f.Id -ErrorAction Stop
      $bases = @($gp.Threads | ForEach-Object { [int]$_.BasePriority } | Group-Object | Sort-Object { [int]$_.Name } | ForEach-Object { '{0} thread(s) at base priority {1}' -f $_.Count, $_.Name })
      $cores = if ($before.ContainsKey($f.Id)) { ($gp.TotalProcessorTime.TotalSeconds - $before[$f.Id]) / [double]$SampleSeconds } else { $null }
      [pscustomobject]@{
        Kind = $f.Kind; Id = $f.Id; PriorityClass = "$($gp.PriorityClass)"; Threads = @($gp.Threads).Count
        BasePriorities = $bases; AllThreadsNormal = (@($gp.Threads | Where-Object { [int]$_.BasePriority -ne 8 }).Count -eq 0)
        CoresBusy = $cores; Affinity = ('0x{0:X}' -f [int64]$gp.ProcessorAffinity); SampleSeconds = $SampleSeconds
      }
    } catch { }
  }
}

# Evidence lines for Get-WbStressInfo: count, priority class and thread priorities of each stress
# process, and whether they are the normal priority the load gate requires.
function Get-WbStressLines {
  param($Info)
  $items = @($Info | Where-Object { $_ })
  if ($items.Count -eq 0) { return @('  stress priority: no stress process was found running (not measured)') }
  $lines = @()
  $bad = 0
  foreach ($i in ($items | Sort-Object Kind)) {
    $label = switch ($i.Kind) { 'job' { 'inside the job' } 'host' { 'on the host, outside the job' } default { 'other' } }
    $cpu = if ($null -ne $i.CoresBusy) { ('{0:n1} cores busy over {1} s' -f $i.CoresBusy, $i.SampleSeconds) } else { 'CPU use not measured' }
    $ok = ($i.PriorityClass -eq 'Normal' -and $i.AllThreadsNormal)
    if (-not $ok) { $bad++ }
    $verdict = if ($ok) { 'normal, as the load gate requires' } else { 'NOT NORMAL PRIORITY' }
    $lines += ("  stress priority, {0}: pid {1}, priority class {2}, {3} threads ({4}), {5}, affinity {6}: {7}" -f $label, $i.Id, $i.PriorityClass, $i.Threads, ($i.BasePriorities -join '; '), $cpu, $i.Affinity, $verdict)
  }
  if ($bad -gt 0) { $lines += '  stress priority: NOT NORMAL for at least one stress process; the load gate requires normal priority, so this load is not the one the gate defines' }
  else { $lines += '  stress priority: every stress process is at normal priority (class Normal, all threads at base priority 8)' }
  return $lines
}

Export-ModuleMember -Function Get-WbStressKind, Get-WbStressInfo, Get-WbStressLines, Get-WbReferenceXrunLines, ConvertFrom-WbMask, Format-WbMask, Format-WbLogicalList, Get-WbRigPlan, Assert-WbRigPlacement, Set-WbRigPlacement, Get-WbProcessCpuSets, Get-WbJobPlacement, Set-WbRigPins, Confirm-WbRigAgainstJob, Get-WbRigLines, Get-WbLateCallbackLines
