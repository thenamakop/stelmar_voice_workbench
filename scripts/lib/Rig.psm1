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
    $lines += "    $($p.Role): pid $($p.ProcessId), mask $(Format-WbMask $p.Mask) (logical processors $(Format-WbLogicalList $p.Mask))$prio$own"
  }
  $lines += '    overlap: none (checked on the planned masks before anything started, and on the masks read back from the running processes)'
  if ($Rig.Plan -and $Rig.Plan.Note) { $lines += "    note: $($Rig.Plan.Note)" }
  $lines
}

# Evidence lines for the player's late render callbacks (a gap between render callbacks longer
# than its 100 ms output buffer, the only kind that can run the buffer dry), and how many fall
# within $WindowMs of each given source position (the shared breaks).
function Get-WbLateCallbackLines {
  param($Play, [double[]]$BreakSourceMs = @(), [double]$WindowMs = 500)
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
      $lines += ("    shared break at source {0:n3} s: {1} late player callback(s) within {2:n0} ms of it" -f ($b / 1000.0), $near.Count, $WindowMs)
    } else {
      $lines += ("    shared break at source {0:n3} s: no late player callback within {1:n0} ms of it; the longest callback gap in the whole run was {2:n1} ms, so the player never ran its output buffer dry" -f ($b / 1000.0), $WindowMs, $maxGap)
    }
  }
  $lines
}

Export-ModuleMember -Function ConvertFrom-WbMask, Format-WbMask, Format-WbLogicalList, Get-WbRigPlan, Assert-WbRigPlacement, Set-WbRigPlacement, Get-WbJobPlacement, Set-WbRigPins, Confirm-WbRigAgainstJob, Get-WbRigLines, Get-WbLateCallbackLines
