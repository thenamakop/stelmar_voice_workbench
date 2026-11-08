<#
  NetworkState.psm1: decides whether this machine is online, for the offline gate (Phase 0) and the
  offline proof (Phase 8).

  A default route alone does not prove connectivity on Windows: routes can stay behind on a
  disconnected Wi-Fi adapter, Hyper-V or WSL vEthernet adapters, VPN or tunnel adapters, and
  IPv6 pseudo-interfaces. The decision therefore uses adapters, connection profiles and, when the
  facts say offline, a short TCP probe.

  Get-NetworkFacts      collects facts without generating network traffic.
  Resolve-NetworkState  pure function: facts (+ probe results) -> State and reasons.
  Test-InternetProbe    TCP connect to fixed IP addresses (no DNS, no HTTP), 2-second timeout.
  Format-NetworkState   lines for the console and the evidence file.

  ASCII only; runs under Windows PowerShell 5.1.
#>

Set-StrictMode -Version 2

$script:ConnectedLevels = @('Internet', 'LocalNetwork')

function Test-NonLinkLocalAddress([string]$Address) {
  if (-not $Address) { return $false }
  if ($Address -like '169.254.*') { return $false }
  if ($Address -like 'fe80:*') { return $false }
  if ($Address -eq '127.0.0.1' -or $Address -eq '::1') { return $false }
  return $true
}

function Get-NetworkFacts {
  [CmdletBinding()]
  param()
  $errors = New-Object System.Collections.Generic.List[string]

  $adapters = @()
  try { $adapters = @(Get-NetAdapter -IncludeHidden -ErrorAction Stop) } catch { $errors.Add("Get-NetAdapter: $($_.Exception.Message)") }

  $routes = @()
  try {
    $routes = @(Get-NetRoute -PolicyStore ActiveStore -DestinationPrefix '0.0.0.0/0', '::/0' -ErrorAction SilentlyContinue |
        ForEach-Object {
          [pscustomobject]@{
            DestinationPrefix = $_.DestinationPrefix
            ifIndex           = [int]$_.ifIndex
            InterfaceAlias    = $_.InterfaceAlias
            NextHop           = $_.NextHop
            RouteMetric       = [int]$_.RouteMetric
          }
        })
  } catch { $errors.Add("Get-NetRoute: $($_.Exception.Message)") }

  $profiles = @()
  try {
    # Returns nothing when no network is connected; that is offline evidence, not an error.
    $profiles = @(Get-NetConnectionProfile -ErrorAction SilentlyContinue | ForEach-Object {
        [pscustomobject]@{
          ifIndex          = [int]$_.InterfaceIndex
          InterfaceAlias   = $_.InterfaceAlias
          IPv4Connectivity = "$($_.IPv4Connectivity)"
          IPv6Connectivity = "$($_.IPv6Connectivity)"
        }
      })
  } catch { $errors.Add("Get-NetConnectionProfile: $($_.Exception.Message)") }

  $ipInterfaces = @(Get-NetIPInterface -ErrorAction SilentlyContinue)
  $addresses = @(Get-NetIPAddress -ErrorAction SilentlyContinue)

  $interfaces = @()
  foreach ($idx in @($routes | ForEach-Object { $_.ifIndex } | Sort-Object -Unique)) {
    $adapter = $adapters | Where-Object { [int]$_.ifIndex -eq $idx } | Select-Object -First 1
    $ipIf = @($ipInterfaces | Where-Object { [int]$_.ifIndex -eq $idx })
    $alias = if ($adapter) { $adapter.Name } elseif ($ipIf.Count) { $ipIf[0].InterfaceAlias } else { "ifIndex $idx" }
    $interfaces += [pscustomobject]@{
      ifIndex          = $idx
      Alias            = $alias
      Description      = if ($adapter) { $adapter.InterfaceDescription } else { '' }
      Present          = [bool]$adapter -or $ipIf.Count -gt 0
      Status           = if ($adapter) { "$($adapter.Status)" } else { 'Not an adapter' }
      Virtual          = if ($adapter) { [bool]$adapter.Virtual } else { $true }
      IsLoopback       = ($alias -like '*Loopback*')
      ConnectionStates = @($ipIf | ForEach-Object { "$($_.AddressFamily):$($_.ConnectionState)" })
      HasUnicast       = @($addresses | Where-Object { [int]$_.InterfaceIndex -eq $idx -and (Test-NonLinkLocalAddress $_.IPAddress) }).Count -gt 0
    }
  }

  $physicalUp = @($adapters | Where-Object { $_.Status -eq 'Up' -and -not $_.Virtual } | ForEach-Object {
      [pscustomobject]@{ ifIndex = [int]$_.ifIndex; Name = $_.Name; Description = $_.InterfaceDescription }
    })

  [pscustomobject]@{
    Routes     = $routes
    Interfaces = $interfaces
    Profiles   = $profiles
    PhysicalUp = $physicalUp
    Errors     = @($errors)
  }
}

function Resolve-NetworkState {
  [CmdletBinding()]
  param(
    [Parameter(Mandatory)] $Facts,
    $ProbeResults = @()
  )
  $online = New-Object System.Collections.Generic.List[string]
  $reasons = New-Object System.Collections.Generic.List[string]
  $ignored = New-Object System.Collections.Generic.List[string]

  foreach ($a in @($Facts.PhysicalUp)) {
    $online.Add("physical adapter Up: $($a.Name) ($($a.Description))")
  }

  $connectedIfs = @{}
  foreach ($p in @($Facts.Profiles)) {
    if ($p.InterfaceAlias -like '*Loopback*') { continue }
    $levels = @()
    if ($script:ConnectedLevels -contains $p.IPv4Connectivity) { $levels += "IPv4 $($p.IPv4Connectivity)" }
    if ($script:ConnectedLevels -contains $p.IPv6Connectivity) { $levels += "IPv6 $($p.IPv6Connectivity)" }
    if ($levels.Count) {
      $connectedIfs[[int]$p.ifIndex] = $true
      $online.Add("connection profile on $($p.InterfaceAlias) reports $($levels -join ', ')")
    }
  }

  foreach ($r in @($ProbeResults)) {
    if ($r.Reached) { $online.Add("probe reached $($r.Target)") }
  }

  foreach ($route in @($Facts.Routes)) {
    $if = @($Facts.Interfaces) | Where-Object { [int]$_.ifIndex -eq [int]$route.ifIndex } | Select-Object -First 1
    $label = "$($route.DestinationPrefix) via $($route.NextHop) on $($route.InterfaceAlias) (ifIndex $($route.ifIndex))"
    if ($connectedIfs.ContainsKey([int]$route.ifIndex)) { continue }
    $why = $null
    if (-not $if -or -not $if.Present) { $why = 'interface not present' }
    elseif ($if.IsLoopback) { $why = 'loopback interface' }
    elseif ($if.Status -ne 'Up') { $why = "adapter is $($if.Status)" }
    elseif (-not $if.HasUnicast) { $why = 'interface has only link-local addresses' }
    elseif ($if.Virtual) { $why = 'virtual adapter with no connected profile' }
    else { $why = 'no connected profile on this interface' }
    $ignored.Add("$label - ignored: $why")
  }

  if ($online.Count) {
    $state = 'Online'
    foreach ($o in $online) { $reasons.Add($o) }
  } elseif (@($Facts.Errors).Count) {
    $state = 'Unknown'
    foreach ($e in @($Facts.Errors)) { $reasons.Add("could not read: $e") }
  } else {
    $state = 'Offline'
    $reasons.Add('no physical adapter (Wi-Fi or Ethernet) is Up')
    $reasons.Add('no connection profile reports Internet or LocalNetwork')
    if (@($ProbeResults).Count) { $reasons.Add('no probe reached its target') }
  }

  [pscustomobject]@{
    State        = $state
    Reasons      = @($reasons)
    Ignored      = @($ignored)
    ProbeResults = @($ProbeResults)
  }
}

function Test-InternetProbe {
  [CmdletBinding()]
  param(
    [string[]]$Targets = @('1.1.1.1:443', '8.8.8.8:443'),
    [int]$TimeoutMs = 2000
  )
  foreach ($t in $Targets) {
    $ip, $port = $t.Split(':')
    $client = New-Object System.Net.Sockets.TcpClient
    $reached = $false; $detail = ''
    try {
      $task = $client.ConnectAsync($ip, [int]$port)
      if ($task.Wait($TimeoutMs)) { $reached = $client.Connected; $detail = 'connected' }
      else { $detail = "no answer within $TimeoutMs ms" }
    } catch {
      $inner = $_.Exception
      while ($inner.InnerException) { $inner = $inner.InnerException }
      $detail = $inner.Message
    } finally {
      $client.Dispose()
    }
    [pscustomobject]@{ Target = $t; Reached = [bool]$reached; Detail = $detail }
  }
}

function Format-NetworkState {
  [CmdletBinding()]
  param([Parameter(Mandatory)] $Facts, [Parameter(Mandatory)] $Result)
  $lines = @()
  $lines += "Network state: $($Result.State)"
  foreach ($r in $Result.Reasons) { $lines += "  reason: $r" }
  if (-not @($Facts.Routes).Count) { $lines += '  default routes (ActiveStore): none' }
  foreach ($route in @($Facts.Routes)) {
    $lines += "  default route: $($route.DestinationPrefix) via $($route.NextHop) on $($route.InterfaceAlias) (ifIndex $($route.ifIndex), metric $($route.RouteMetric))"
  }
  foreach ($i in $Result.Ignored) { $lines += "  $i" }
  foreach ($if in @($Facts.Interfaces)) {
    $lines += "  interface $($if.Alias) (ifIndex $($if.ifIndex)): status $($if.Status), virtual $($if.Virtual), $($if.ConnectionStates -join ' '), non-link-local address $($if.HasUnicast)"
  }
  if (-not @($Facts.Profiles).Count) { $lines += '  connection profiles: none' }
  foreach ($p in @($Facts.Profiles)) {
    $lines += "  connection profile: $($p.InterfaceAlias) IPv4 $($p.IPv4Connectivity), IPv6 $($p.IPv6Connectivity)"
  }
  if (-not @($Facts.PhysicalUp).Count) { $lines += '  physical adapters Up: none' }
  foreach ($a in @($Facts.PhysicalUp)) { $lines += "  physical adapter Up: $($a.Name) ($($a.Description))" }
  if (@($Result.ProbeResults).Count) {
    foreach ($p in $Result.ProbeResults) { $lines += "  probe $($p.Target): reached $($p.Reached) ($($p.Detail))" }
  } else {
    $lines += '  probe: not run (facts already show the machine online)'
  }
  $lines
}

Export-ModuleMember -Function Get-NetworkFacts, Resolve-NetworkState, Test-InternetProbe, Format-NetworkState
