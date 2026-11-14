<#
  PeImports.psm1: reads the import tables of Windows executables with no development tool (no
  dumpbin), so the gate pack can show on a laptop that no shipped binary links the network libraries.

  Get-PePeImports reads the import table and the delay-load import table of a PE32 or PE32+ file.
  Test-PackNetworkImports runs it over a list of files, checks the reader against a positive control
  (a Windows program that is known to import Winsock), and says which files import which of
  Winsock (ws2_32, wsock32, mswsock), WinHTTP or WinINet.

  A file that imports one of those does not necessarily open a connection (a library can link
  Winsock and never call it). The gate reports it as a finding; it does not decide on the owner's
  behalf that it is harmless.

  Windows PowerShell 5.1, ASCII only.
#>

$script:NetworkDlls = @('ws2_32.dll', 'wsock32.dll', 'mswsock.dll', 'winhttp.dll', 'wininet.dll')

function Get-PeSectionOffset {
  param($Sections, [uint32]$Rva)
  foreach ($s in $Sections) {
    $size = [math]::Max([uint32]$s.VirtualSize, [uint32]$s.RawSize)
    if ($Rva -ge $s.VirtualAddress -and $Rva -lt ($s.VirtualAddress + $size)) { return [int64]($Rva - $s.VirtualAddress + $s.RawPointer) }
  }
  -1
}

function Read-PeCString {
  param([byte[]]$Bytes, [int64]$Offset)
  if ($Offset -lt 0 -or $Offset -ge $Bytes.Length) { return $null }
  $end = $Offset
  while ($end -lt $Bytes.Length -and $Bytes[$end] -ne 0 -and ($end - $Offset) -lt 260) { $end++ }
  [System.Text.Encoding]::ASCII.GetString($Bytes, [int]$Offset, [int]($end - $Offset))
}

# Names of the DLLs a PE file imports: Dll, Kind (import or delay). Returns $null when the file is
# not a PE file.
function Get-PePeImports {
  [CmdletBinding()]
  param([Parameter(Mandatory = $true)] [string]$Path)
  $b = [System.IO.File]::ReadAllBytes($Path)
  if ($b.Length -lt 0x100 -or $b[0] -ne 0x4D -or $b[1] -ne 0x5A) { return $null }
  $pe = [BitConverter]::ToInt32($b, 0x3C)
  if ($pe -lt 0 -or $pe + 24 -gt $b.Length) { return $null }
  if ($b[$pe] -ne 0x50 -or $b[$pe + 1] -ne 0x45) { return $null }
  $nSections = [BitConverter]::ToUInt16($b, $pe + 6)
  $optSize = [BitConverter]::ToUInt16($b, $pe + 20)
  $opt = $pe + 24
  $magic = [BitConverter]::ToUInt16($b, $opt)
  $ddOffset = $(if ($magic -eq 0x20B) { $opt + 112 } elseif ($magic -eq 0x10B) { $opt + 96 } else { return $null })
  $imageBase = $(if ($magic -eq 0x20B) { [BitConverter]::ToUInt64($b, $opt + 24) } else { [uint64][BitConverter]::ToUInt32($b, $opt + 28) })
  $sectionTable = $opt + $optSize
  $sections = New-Object System.Collections.Generic.List[object]
  for ($i = 0; $i -lt $nSections; $i++) {
    $o = $sectionTable + 40 * $i
    if ($o + 40 -gt $b.Length) { break }
    $sections.Add([pscustomobject]@{ VirtualSize = [BitConverter]::ToUInt32($b, $o + 8); VirtualAddress = [BitConverter]::ToUInt32($b, $o + 12); RawSize = [BitConverter]::ToUInt32($b, $o + 16); RawPointer = [BitConverter]::ToUInt32($b, $o + 20) })
  }
  $found = New-Object System.Collections.Generic.List[object]
  # Import directory (index 1): 20-byte descriptors, name RVA at offset 12, ends with an all-zero one.
  $impRva = [BitConverter]::ToUInt32($b, $ddOffset + 8)
  if ($impRva -ne 0) {
    $off = Get-PeSectionOffset -Sections $sections -Rva $impRva
    $n = 0
    while ($off -ge 0 -and $off + 20 -le $b.Length -and $n -lt 512) {
      $nameRva = [BitConverter]::ToUInt32($b, [int]$off + 12)
      $first = [BitConverter]::ToUInt32($b, [int]$off + 16)
      if ($nameRva -eq 0 -and $first -eq 0) { break }
      $name = Read-PeCString -Bytes $b -Offset (Get-PeSectionOffset -Sections $sections -Rva $nameRva)
      if ($name) { $found.Add([pscustomobject]@{ Dll = $name.ToLowerInvariant(); Kind = 'import' }) }
      $off += 20; $n++
    }
  }
  # Delay-load directory (index 13): 32-byte descriptors, attributes at 0, name at 4.
  $delayRva = [BitConverter]::ToUInt32($b, $ddOffset + 13 * 8)
  if ($delayRva -ne 0) {
    $off = Get-PeSectionOffset -Sections $sections -Rva $delayRva
    $n = 0
    while ($off -ge 0 -and $off + 32 -le $b.Length -and $n -lt 512) {
      $attr = [BitConverter]::ToUInt32($b, [int]$off)
      $nameRef = [BitConverter]::ToUInt32($b, [int]$off + 4)
      if ($attr -eq 0 -and $nameRef -eq 0) { break }
      $rva = $nameRef
      if (($attr -band 1) -eq 0 -and $nameRef -ge $imageBase) { $rva = [uint32]($nameRef - $imageBase) }
      $name = Read-PeCString -Bytes $b -Offset (Get-PeSectionOffset -Sections $sections -Rva $rva)
      if ($name) { $found.Add([pscustomobject]@{ Dll = $name.ToLowerInvariant(); Kind = 'delay' }) }
      $off += 32; $n++
    }
  }
  @($found.ToArray())
}

# The network libraries among a list of imports (pure).
function Select-PackNetworkDlls {
  param($Imports)
  @(@($Imports) | Where-Object { $script:NetworkDlls -contains $_.Dll } | ForEach-Object { $_.Dll } | Select-Object -Unique)
}

# Runs the reader over files. Returns Control (did the reader see Winsock/WinHTTP/WinINet in a program
# that is known to use them), Results (one per file: Path, Imports, Network, Error) and Findings.
function Test-PackNetworkImports {
  [CmdletBinding()]
  param([Parameter(Mandatory = $true)] [string[]]$Files, [string[]]$ControlFiles = @())
  if ($ControlFiles.Count -eq 0) {
    $sys = Join-Path $env:SystemRoot 'System32'
    $ControlFiles = @((Join-Path $sys 'netstat.exe'), (Join-Path $sys 'curl.exe'), (Join-Path $sys 'ping.exe'))
  }
  $controlHit = $null
  foreach ($c in $ControlFiles) {
    if (-not (Test-Path -LiteralPath $c)) { continue }
    try {
      $imp = Get-PePeImports -Path $c
      $net = @(Select-PackNetworkDlls $imp)
      if ($net.Count -gt 0) { $controlHit = [pscustomobject]@{ Path = $c; Dlls = $net; ImportCount = @($imp).Count }; break }
    } catch { }
  }
  $results = New-Object System.Collections.Generic.List[object]
  foreach ($f in $Files) {
    $imports = $null; $err = ''
    try { $imports = Get-PePeImports -Path $f } catch { $err = $_.Exception.Message }
    if ($null -eq $imports -and -not $err) { $err = 'not a PE file' }
    $all = @($imports | ForEach-Object { $_.Dll } | Select-Object -Unique)
    $results.Add([pscustomobject]@{ Path = $f; Imports = $all; Network = @(Select-PackNetworkDlls $imports); Error = $err })
  }
  [pscustomobject]@{
    ControlOk = ($null -ne $controlHit)
    Control = $controlHit
    Results = @($results.ToArray())
    Findings = @($results.ToArray() | Where-Object { @($_.Network).Count -gt 0 })
    Unreadable = @($results.ToArray() | Where-Object { $_.Error })
  }
}

Export-ModuleMember -Function Get-PePeImports, Select-PackNetworkDlls, Test-PackNetworkImports
