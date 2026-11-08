# Tests for scripts/lib/NetworkState.psm1 (Resolve-NetworkState only; it is pure).
# Run: powershell -NoProfile -ExecutionPolicy Bypass -File scripts/tests/NetworkState.Tests.ps1
# No Pester needed. Exit code 1 when any case fails. ASCII only; Windows PowerShell 5.1.
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
Import-Module (Join-Path $here '..\lib\NetworkState.psm1') -Force

$script:failures = 0
$script:passes = 0
function Assert-Equal($Expected, $Actual, [string]$What) {
  if ($Expected -ne $Actual) { throw "$What : expected '$Expected', got '$Actual'" }
}
function Assert-Match([string[]]$Lines, [string]$Pattern, [string]$What) {
  if (-not (@($Lines) | Where-Object { $_ -match $Pattern })) { throw "$What : no line matches '$Pattern' in [$($Lines -join ' | ')]" }
}
function Test-Case([string]$Name, [scriptblock]$Body) {
  try { & $Body; $script:passes++; Write-Host "PASS  $Name" }
  catch { $script:failures++; Write-Host "FAIL  $Name : $($_.Exception.Message)" }
}

function New-Route([string]$Prefix, [int]$Index, [string]$Alias, [string]$NextHop) {
  [pscustomobject]@{ DestinationPrefix = $Prefix; ifIndex = $Index; InterfaceAlias = $Alias; NextHop = $NextHop; RouteMetric = 0 }
}
function New-Interface([int]$Index, [string]$Alias, [string]$Status, [bool]$Virtual, [bool]$HasUnicast, [bool]$Present = $true) {
  [pscustomobject]@{
    ifIndex = $Index; Alias = $Alias; Description = "$Alias adapter"; Present = $Present; Status = $Status
    Virtual = $Virtual; IsLoopback = $false; ConnectionStates = @(); HasUnicast = $HasUnicast
  }
}
function New-Profile([int]$Index, [string]$Alias, [string]$V4, [string]$V6 = 'NoTraffic') {
  [pscustomobject]@{ ifIndex = $Index; InterfaceAlias = $Alias; IPv4Connectivity = $V4; IPv6Connectivity = $V6 }
}
function New-Facts($Routes = @(), $Interfaces = @(), $Profiles = @(), $PhysicalUp = @()) {
  [pscustomobject]@{ Routes = @($Routes); Interfaces = @($Interfaces); Profiles = @($Profiles); PhysicalUp = @($PhysicalUp); Errors = @() }
}

Test-Case 'a) stale default route on a disconnected Wi-Fi adapter, nothing Up -> Offline' {
  # The Legion on 8 Nov 2026: 0.0.0.0/0 via 192.168.1.1 on Wi-Fi (ifIndex 6) with Wi-Fi Disconnected.
  $facts = New-Facts -Routes @(New-Route '0.0.0.0/0' 6 'Wi-Fi' '192.168.1.1') `
    -Interfaces @(New-Interface 6 'Wi-Fi' 'Disconnected' $false $false)
  $r = Resolve-NetworkState -Facts $facts
  Assert-Equal 'Offline' $r.State 'state'
  Assert-Match $r.Ignored 'Wi-Fi .*ignored: adapter is Disconnected' 'ignored route'
}

Test-Case 'b) Hyper-V/WSL vEthernet Up with a default route but no connected profile -> Offline' {
  $facts = New-Facts -Routes @(New-Route '0.0.0.0/0' 40 'vEthernet (WSL)' '172.20.0.1') `
    -Interfaces @(New-Interface 40 'vEthernet (WSL)' 'Up' $true $true)
  $r = Resolve-NetworkState -Facts $facts
  Assert-Equal 'Offline' $r.State 'state'
  Assert-Match $r.Ignored 'vEthernet \(WSL\).*ignored: virtual adapter with no connected profile' 'ignored route'
}

Test-Case 'c) Wi-Fi Up with a profile reporting Internet -> Online' {
  $wifi = [pscustomobject]@{ ifIndex = 6; Name = 'Wi-Fi'; Description = 'Intel(R) Wi-Fi 6E AX211 160MHz' }
  $facts = New-Facts -Routes @(New-Route '0.0.0.0/0' 6 'Wi-Fi' '192.168.1.1') `
    -Interfaces @(New-Interface 6 'Wi-Fi' 'Up' $false $true) `
    -Profiles @(New-Profile 6 'Wi-Fi' 'Internet') -PhysicalUp @($wifi)
  $r = Resolve-NetworkState -Facts $facts
  Assert-Equal 'Online' $r.State 'state'
  Assert-Match $r.Reasons 'physical adapter Up: Wi-Fi' 'adapter reason'
  Assert-Match $r.Reasons 'connection profile on Wi-Fi reports IPv4 Internet' 'profile reason'
  Assert-Equal 0 @($r.Ignored).Count 'ignored routes'
}

Test-Case 'd) VPN/tunnel adapter with a connected profile reporting Internet -> Online' {
  $facts = New-Facts -Routes @(New-Route '0.0.0.0/0' 25 'Corp VPN' '10.8.0.1') `
    -Interfaces @(New-Interface 25 'Corp VPN' 'Up' $true $true) `
    -Profiles @(New-Profile 25 'Corp VPN' 'Internet')
  $r = Resolve-NetworkState -Facts $facts
  Assert-Equal 'Online' $r.State 'state'
  Assert-Match $r.Reasons 'connection profile on Corp VPN reports IPv4 Internet' 'profile reason'
}

Test-Case 'e) no physical adapter Up, but a probe succeeded -> Online' {
  $facts = New-Facts -Routes @(New-Route '0.0.0.0/0' 6 'Wi-Fi' '192.168.1.1') `
    -Interfaces @(New-Interface 6 'Wi-Fi' 'Disconnected' $false $false)
  $probes = @(
    [pscustomobject]@{ Target = '1.1.1.1:443'; Reached = $false; Detail = 'no answer within 2000 ms' },
    [pscustomobject]@{ Target = '8.8.8.8:443'; Reached = $true; Detail = 'connected' }
  )
  $r = Resolve-NetworkState -Facts $facts -ProbeResults $probes
  Assert-Equal 'Online' $r.State 'state'
  Assert-Match $r.Reasons 'probe reached 8.8.8.8:443' 'probe reason'
}

Test-Case 'f) IPv6 ::/0 on a Teredo/6to4 pseudo-interface with no connectivity -> Offline' {
  $facts = New-Facts -Routes @(New-Route '::/0' 14 'Teredo Tunneling Pseudo-Interface' '::') `
    -Interfaces @(New-Interface 14 'Teredo Tunneling Pseudo-Interface' 'Not Present' $true $false) `
    -Profiles @(New-Profile 14 'Teredo Tunneling Pseudo-Interface' 'NoTraffic' 'NoTraffic')
  $r = Resolve-NetworkState -Facts $facts
  Assert-Equal 'Offline' $r.State 'state'
  Assert-Match $r.Ignored 'Teredo.*ignored' 'ignored route'
}

Test-Case 'g) failed probes with offline facts stay Offline and say so' {
  $facts = New-Facts
  $probes = @([pscustomobject]@{ Target = '1.1.1.1:443'; Reached = $false; Detail = 'unreachable' })
  $r = Resolve-NetworkState -Facts $facts -ProbeResults $probes
  Assert-Equal 'Offline' $r.State 'state'
  Assert-Match $r.Reasons 'no probe reached' 'probe reason'
}

Write-Host ""
Write-Host "NetworkState tests: $script:passes passed, $script:failures failed"
if ($script:failures -gt 0) { exit 1 }
