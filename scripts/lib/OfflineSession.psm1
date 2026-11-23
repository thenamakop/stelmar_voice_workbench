<#
  OfflineSession.psm1: the pure logic of scripts/offline-session-check.ps1, tested without a network or the app
  (scripts/tests/OfflineSession.Tests.ps1).

  1. ConvertTo-WbPrivateMasked: replaces private network addresses in TEXT with <private-ip>. It never decides
     anything: the check classifies the raw addresses first (a non-loopback socket is a FAIL whatever it is), and
     only the text that is printed or written to docs/evidence is masked, because the repository is public and a
     home or office address is private information. Masked: IPv4 10/8, 172.16/12, 192.168/16, 169.254/16; IPv6
     fc00::/7 (unique local) and fe80::/10 (link local), with or without a %zone id, and IPv4-mapped IPv6 of an
     IPv4 range above. Loopback, 0.0.0.0, :: and public addresses are left as they are.

  2. Get-WbEngineSubcommand: which subcommand an engine process was started with (info, bench, batch, live).

  3. Get-WbOfflineVerdict: the verdict of an offline run. A pass needs proof that the app did its job offline
     (capability.json in the app data folder, a transcript.db in the session folder, and an engine process doing
     post-processing, that is `batch`, in the watched tree), not only the absence of sockets: a run in which
     nothing was processed proves nothing (the first attempt of 22 Nov 2026 was such a run).

  ASCII only; Windows PowerShell 5.1.
#>
Set-StrictMode -Version 2

$script:Mask = '<private-ip>'

function Test-WbPrivateAddress {
  [CmdletBinding()]
  param([Parameter(Mandatory = $true)] [string]$Address)
  $text = $Address.Trim('[', ']')
  $zone = $text.IndexOf('%')
  if ($zone -ge 0) { $text = $text.Substring(0, $zone) }
  $ip = $null
  if (-not [System.Net.IPAddress]::TryParse($text, [ref]$ip)) { return $false }
  if ($ip.IsIPv4MappedToIPv6) { $ip = $ip.MapToIPv4() }
  $b = $ip.GetAddressBytes()
  if ($b.Length -eq 4) {
    if ($b[0] -eq 10) { return $true }
    if ($b[0] -eq 172 -and $b[1] -ge 16 -and $b[1] -le 31) { return $true }
    if ($b[0] -eq 192 -and $b[1] -eq 168) { return $true }
    if ($b[0] -eq 169 -and $b[1] -eq 254) { return $true }
    return $false
  }
  if (($b[0] -band 0xFE) -eq 0xFC) { return $true }                        # fc00::/7
  if ($b[0] -eq 0xFE -and ($b[1] -band 0xC0) -eq 0x80) { return $true }   # fe80::/10
  $false
}

# IPv4 first (a dotted quad that is not part of a longer dotted number), then IPv6 candidates (hex groups
# separated by colons, with an optional %zone and an optional dotted tail); every candidate is parsed before it
# is masked, so times such as 10:00:00 and version numbers are left alone.
$script:V4 = [regex]'(?<![\d.])\d{1,3}(?:\.\d{1,3}){3}(?![\d]|\.\d)'
$script:V6 = [regex]'(?<![0-9A-Za-z:.])(?:[0-9A-Fa-f]{0,4}:){2,7}(?:(?:\d{1,3}\.){3}\d{1,3}|[0-9A-Fa-f]{1,4})?(?:%[0-9A-Za-z_.-]+)?(?![0-9A-Za-z:]|\.\d)|(?<![0-9A-Za-z:.])::(?![0-9A-Za-z:])'

function ConvertTo-WbPrivateMasked {
  [CmdletBinding()]
  param([Parameter(ValueFromPipeline = $true)] [AllowEmptyString()] [AllowNull()] [string[]]$Text)
  process {
    foreach ($t in $Text) {
      if ($null -eq $t) { continue }
      $s = $t
      # IPv6 first: an IPv4-mapped address ends in a dotted quad that the IPv4 pass would otherwise cut out of it.
      $s = $script:V6.Replace($s, {
          param($m)
          # "fe80::1:443" (an address and a port joined by a colon) parses as one address; a bracketed form is
          # used by the check, so this only matters for text from elsewhere.
          if (Test-WbPrivateAddress -Address $m.Value) { $script:Mask } else { $m.Value }
        })
      $s = $script:V4.Replace($s, {
          param($m)
          if (Test-WbPrivateAddress -Address $m.Value) { $script:Mask } else { $m.Value }
        })
      $s
    }
  }
}

# Brackets an IPv6 address so that "address:port" cannot be mistaken for a longer address: [fe80::1%12]:443.
function Format-WbEndpoint {
  [CmdletBinding()]
  param([Parameter(Mandatory = $true)] [string]$Address, [Parameter(Mandatory = $true)] [int]$Port)
  if ($Address.Contains(':')) { "[$Address]:$Port" } else { "$($Address):$Port" }
}

# The subcommand of an engine process from its command line: the first word after the program.
function Get-WbEngineSubcommand {
  [CmdletBinding()]
  param([AllowNull()] [AllowEmptyString()] [string]$CommandLine)
  if (-not $CommandLine) { return $null }
  $m = [regex]::Match($CommandLine, '(?i)wb-engine[^\s"]*\.exe"?\s+([A-Za-z][A-Za-z0-9_-]*)')
  if ($m.Success) { $m.Groups[1].Value.ToLowerInvariant() } else { $null }
}

# The verdict of an offline run. Returns Verdict (the line to print) and ExitCode.
#   A non-loopback socket seen, a gate string in the exe, or an app that exited early is a FAIL (exit 1), whatever
#   else is true. Otherwise every proof of post-processing must be there, or the run is INCONCLUSIVE (exit 4) and
#   says which proof is missing.
function Get-WbOfflineVerdict {
  [CmdletBinding()]
  param(
    [Parameter(Mandatory = $true)] [int]$NonLoopbackSeen,
    [Parameter(Mandatory = $true)] [int]$GateStrings,
    [Parameter(Mandatory = $true)] [bool]$AppAlive,
    [Parameter(Mandatory = $true)] [bool]$CapabilityJson,
    [Parameter(Mandatory = $true)] [bool]$TranscriptDb,
    [Parameter(Mandatory = $true)] [bool]$EngineBeyondProbes
  )
  if ($NonLoopbackSeen -gt 0 -or $GateStrings -gt 0 -or -not $AppAlive) {
    return [pscustomobject]@{ Verdict = 'verdict: FAIL'; ExitCode = 1; Missing = @() }
  }
  $missing = New-Object System.Collections.Generic.List[string]
  if (-not $CapabilityJson) { $missing.Add('capability.json missing: the first-launch system check has not completed') }
  if (-not $TranscriptDb) { $missing.Add('no transcript.db in the session folder') }
  if (-not $EngineBeyondProbes) { $missing.Add('no engine process beyond the start-up probes (no batch run) seen in the watched tree') }
  if ($missing.Count -gt 0) {
    return [pscustomobject]@{ Verdict = "verdict: INCONCLUSIVE (no post-processing: $($missing -join '; '))"; ExitCode = 4; Missing = @($missing) }
  }
  [pscustomobject]@{
    Verdict  = 'verdict: PASS (offline: no non-loopback socket while a recording session was post-processed: capability.json, transcript.db and a batch engine all present)'
    ExitCode = 0
    Missing  = @()
  }
}

Export-ModuleMember -Function ConvertTo-WbPrivateMasked, Test-WbPrivateAddress, Format-WbEndpoint, Get-WbEngineSubcommand, Get-WbOfflineVerdict
