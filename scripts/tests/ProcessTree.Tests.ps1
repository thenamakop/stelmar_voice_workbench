# Tests for scripts/lib/ProcessTree.psm1 (the engine tree for the network-off socket check).
# Run: powershell -NoProfile -ExecutionPolicy Bypass -File scripts/tests/ProcessTree.Tests.ps1
# No Pester needed. Exit code 1 when any case fails. ASCII only; Windows PowerShell 5.1.
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
Import-Module (Join-Path $here '..\lib\ProcessTree.psm1') -Force
$script:failures = 0
function Test-Case([string]$Name, [scriptblock]$Body) {
  try { & $Body; Write-Host "PASS  $Name" } catch { $script:failures++; Write-Host "FAIL  $Name : $($_.Exception.Message)" }
}
$t0 = [datetime]'2026-11-11 12:00:00'
function P([int]$Id, [int]$Parent, [double]$AtSeconds, [string]$Name = 'x.exe') {
  [pscustomobject]@{ ProcessId = $Id; ParentProcessId = $Parent; Name = $Name; ExecutablePath = "C:\fake\$Name"; CreationDate = $t0.AddSeconds($AtSeconds) }
}
# The engine is pid 100, created at +100 s. Pid 200 is an old long-lived process (created at +1 s) whose parent id is 100
# because that id belonged to a parent that exited long ago and was reused by the engine. Pid 500 is its child.
$procs = @(
  (P 100 50 100 'wb-engine.exe'),
  (P 300 100 101 'child.exe'),     # a real child of the engine
  (P 400 300 102 'grandchild.exe'),# a real grandchild
  (P 200 100 1 'claude-like.exe'), # stale parent id equal to the engine's pid, created BEFORE the engine
  (P 500 200 2 'its-child.exe'),   # child of the stale process
  (P 600 999 3 'unrelated.exe')
)

Test-Case 'a fake process whose stale parent pid equals the engine pid is not in the tree' {
  $ids = @(Get-WbTreeIds -Processes $procs -RootPid 100)
  if ($ids -contains 200) { throw 'the stale process (200) was counted' }
  if ($ids -contains 500) { throw 'the stale process''s child (500) was counted' }
}
Test-Case 'real children and grandchildren are in the tree, and the root itself' {
  $ids = @(Get-WbTreeIds -Processes $procs -RootPid 100)
  foreach ($want in 100, 300, 400) { if ($ids -notcontains $want) { throw "pid $want missing" } }
  if ($ids -contains 600) { throw 'an unrelated process was counted' }
}
Test-Case 'control: the old rule (parent id alone) does count the stale process, so the test can fail' {
  $ids = @(Get-WbTreeIds -Processes $procs -RootPid 100 -IgnoreCreationTime)
  if ($ids -notcontains 200 -or $ids -notcontains 500) { throw 'the old rule should have counted 200 and 500' }
}
Test-Case 'a real engine connection still counts: a child created after the engine is in the tree whatever address it uses' {
  $more = $procs + @((P 700 300 105 'late-child.exe'))
  $ids = @(Get-WbTreeIds -Processes $more -RootPid 100)
  if ($ids -notcontains 700) { throw 'a child started after the engine was left out' }
}
Test-Case 'a process with an unknown creation time is kept (the safe side)' {
  $unk = [pscustomobject]@{ ProcessId = 800; ParentProcessId = 100; Name = 'u.exe'; ExecutablePath = ''; CreationDate = $null }
  $ids = @(Get-WbTreeIds -Processes ($procs + @($unk)) -RootPid 100)
  if ($ids -notcontains 800) { throw 'unknown creation time was dropped' }
}
Test-Case 'the owner description names pid, name, path, creation time and parent' {
  $d = Get-WbOwnerDescription -Processes $procs -OwnerPid 200
  foreach ($frag in 'pid 200', 'claude-like.exe', 'C:\fake\claude-like.exe', 'created 2026-11-11 12:00:01.000', 'parent pid 100') { if ($d -notlike "*$frag*") { throw "missing '$frag' in: $d" } }
}

# ---- Get-WbSocketVerdict: the socket filter of scripts/gates/network-off.ps1 ----
$start = [datetime]'2026-11-11 12:10:00'
Test-Case 'a TCP listener on 0.0.0.0 or :: is NOT loopback' {
  foreach ($a in '0.0.0.0', '::') {
    $v = Get-WbSocketVerdict -Kind TCP -State Listen -LocalAddress $a -LocalPort 50052 -RemoteAddress '0.0.0.0' -RemotePort 0 -CreationTime $start.AddSeconds(1) -RootStart $start
    if ($v.Verdict -ne 'flag' -or $v.Key -ne "TCP listen $($a):50052") { throw "wildcard listener on $a gave $($v.Verdict) / $($v.Key)" }
  }
}
Test-Case 'a UDP endpoint bound to 0.0.0.0 or :: is NOT loopback' {
  foreach ($a in '0.0.0.0', '::') {
    $v = Get-WbSocketVerdict -Kind UDP -LocalAddress $a -LocalPort 5353 -CreationTime $start.AddSeconds(1) -RootStart $start
    if ($v.Verdict -ne 'flag') { throw "wildcard UDP on $a gave $($v.Verdict)" }
  }
}
Test-Case 'loopback listeners, endpoints and connections are only counted' {
  foreach ($a in '127.0.0.1', '::1') {
    $l = Get-WbSocketVerdict -Kind TCP -State Listen -LocalAddress $a -LocalPort 1 -RemoteAddress '0.0.0.0'
    $u = Get-WbSocketVerdict -Kind UDP -LocalAddress $a -LocalPort 2
    $c = Get-WbSocketVerdict -Kind TCP -State Established -LocalAddress $a -LocalPort 3 -RemoteAddress $a -RemotePort 4
    foreach ($x in $l, $u, $c) { if ($x.Verdict -ne 'loopback') { throw "loopback $a gave $($x.Verdict)" } }
  }
}
Test-Case 'an established connection to a public address is flagged with its remote address and state' {
  $v = Get-WbSocketVerdict -Kind TCP -State Established -LocalAddress '192.168.1.5' -LocalPort 50000 -RemoteAddress '104.18.39.21' -RemotePort 443 -CreationTime $start.AddSeconds(2) -RootStart $start
  if ($v.Verdict -ne 'flag' -or $v.Key -ne 'TCP 104.18.39.21:443 Established') { throw "$($v.Verdict) / $($v.Key)" }
}
Test-Case 'a bound, not connected TCP socket on a non-loopback address is flagged' {
  $v = Get-WbSocketVerdict -Kind TCP -State Bound -LocalAddress '192.168.1.5' -LocalPort 50001 -RemoteAddress '0.0.0.0' -RemotePort 0
  if ($v.Verdict -ne 'flag') { throw $v.Verdict }
  $w = Get-WbSocketVerdict -Kind TCP -State Bound -LocalAddress '127.0.0.1' -LocalPort 50001 -RemoteAddress '0.0.0.0' -RemotePort 0
  if ($w.Verdict -ne 'loopback') { throw "loopback bound gave $($w.Verdict)" }
}
Test-Case 'a socket created more than 2 s before the engine started is listed, not counted; one created after still counts' {
  $old = Get-WbSocketVerdict -Kind TCP -State Established -LocalAddress '192.168.1.5' -LocalPort 50000 -RemoteAddress '104.18.39.21' -RemotePort 443 -CreationTime $start.AddSeconds(-60) -RootStart $start
  if ($old.Verdict -ne 'predates') { throw "old socket gave $($old.Verdict)" }
  $near = Get-WbSocketVerdict -Kind TCP -State Established -LocalAddress '192.168.1.5' -LocalPort 50000 -RemoteAddress '104.18.39.21' -RemotePort 443 -CreationTime $start.AddSeconds(-1) -RootStart $start
  if ($near.Verdict -ne 'flag') { throw "a socket 1 s before the start must still count, got $($near.Verdict)" }
  $unk = Get-WbSocketVerdict -Kind TCP -State Established -LocalAddress '192.168.1.5' -LocalPort 50000 -RemoteAddress '104.18.39.21' -RemotePort 443
  if ($unk.Verdict -ne 'flag') { throw "an unknown creation time must count, got $($unk.Verdict)" }
}
if ($script:failures) { exit 1 }
