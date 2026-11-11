<#
  ProcessTree.psm1: the process tree of a process, safe against parent-PID reuse.

  Windows reuses process ids, and a process keeps the id of its parent even after the parent has exited.
  So "every process whose ParentProcessId is in the set" can pick up an unrelated, older process whose stale
  parent id happens to equal the id of a new process (an engine, for example). A real child is always created
  AFTER its parent, so a process counts as a child only when its creation time is not earlier than its parent's.
  A process whose creation time (or its parent's) is unknown is kept in the tree (the safe side for a check
  that must not miss anything) and is marked.

  ASCII only; Windows PowerShell 5.1.
#>
Set-StrictMode -Version 2

# All processes with the fields the tree and the owner report need.
function Get-WbProcessSnapshot {
  @(Get-CimInstance Win32_Process -ErrorAction SilentlyContinue |
      Select-Object ProcessId, ParentProcessId, Name, ExecutablePath, CreationDate)
}

# The ids of $RootPid and of all its descendants, from a snapshot (objects with ProcessId, ParentProcessId,
# CreationDate). A child is accepted only when it was created at or after its parent.
function Get-WbTreeIds {
  param([Parameter(Mandatory = $true)] $Processes, [Parameter(Mandatory = $true)] [int]$RootPid,
    # Information only: the old rule (parent id alone), to list what the creation-time rule left out.
    [switch]$IgnoreCreationTime)
  $byId = @{}
  foreach ($p in @($Processes)) { $byId[[int]$p.ProcessId] = $p }
  $ids = New-Object System.Collections.Generic.HashSet[int]
  [void]$ids.Add($RootPid)
  $grew = $true
  while ($grew) {
    $grew = $false
    foreach ($p in @($Processes)) {
      $id = [int]$p.ProcessId
      if ($ids.Contains($id)) { continue }
      $parentId = [int]$p.ParentProcessId
      if (-not $ids.Contains($parentId)) { continue }
      $parent = $byId[$parentId]
      $childAt = $p.CreationDate
      $parentAt = $(if ($parent) { $parent.CreationDate } else { $null })
      if (-not $IgnoreCreationTime -and $childAt -and $parentAt -and ([datetime]$childAt -lt [datetime]$parentAt)) { continue }   # older than its "parent": a reused id
      if ($ids.Add($id)) { $grew = $true }
    }
  }
  @($ids)
}

# One line that identifies the owner of a socket: pid, name, path, creation time, parent pid and the parent's creation time.
function Get-WbOwnerDescription {
  param([Parameter(Mandatory = $true)] $Processes, [Parameter(Mandatory = $true)] [int]$OwnerPid)
  $inv = [Globalization.CultureInfo]::InvariantCulture
  $o = @($Processes) | Where-Object { [int]$_.ProcessId -eq $OwnerPid } | Select-Object -First 1
  if (-not $o) { return "owner pid $OwnerPid (not in the process list any more)" }
  $par = @($Processes) | Where-Object { [int]$_.ProcessId -eq [int]$o.ParentProcessId } | Select-Object -First 1
  $created = $(if ($o.CreationDate) { ([datetime]$o.CreationDate).ToString('yyyy-MM-dd HH:mm:ss.fff', $inv) } else { 'unknown' })
  $parText = $(if ($par) { "$($par.Name), created $(if ($par.CreationDate) { ([datetime]$par.CreationDate).ToString('yyyy-MM-dd HH:mm:ss.fff', $inv) } else { 'unknown' })" } else { 'not running' })
  "owner pid $OwnerPid $($o.Name) path $($o.ExecutablePath) created $created, parent pid $($o.ParentProcessId) ($parText)"
}

Export-ModuleMember -Function Get-WbProcessSnapshot, Get-WbTreeIds, Get-WbOwnerDescription
