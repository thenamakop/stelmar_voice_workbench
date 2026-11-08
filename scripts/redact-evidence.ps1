<#
.SYNOPSIS
  Redacts evidence files in place (scripts/lib/Redact.psm1 rules): drops environment blocks and
  replaces the user profile path, user name and computer name. Use it on any file written into
  docs/evidence/ by hand, for example cdb or Get-DnsClientCache output, before committing.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File .\scripts\redact-evidence.ps1 -MachineLabel Legion docs\evidence\phase-0\webview2-dump-analysis.txt
#>
[CmdletBinding()]
param(
  [Parameter(Mandatory = $true, ValueFromRemainingArguments = $true)] [string[]]$Path,
  # Name to show instead of the computer name, for example Legion (docs/GATES.md machine names).
  [string]$MachineLabel = '%COMPUTERNAME%'
)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
Import-Module (Join-Path $here 'lib\Redact.psm1') -Force
foreach ($p in $Path) {
  foreach ($f in @(Get-Item $p)) {
    $before = [IO.File]::ReadAllLines($f.FullName)
    Write-WbEvidence -Path $f.FullName -Lines $before -MachineLabel $MachineLabel
    $after = [IO.File]::ReadAllLines($f.FullName)
    Write-Host ("{0}: {1} -> {2} lines" -f $f.Name, $before.Count, $after.Count)
  }
}
