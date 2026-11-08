<#
  Redact.psm1: removes personal and secret data from text before it is written to docs/evidence/
  (the repository is public). Every evidence writer uses Write-WbEvidence or ConvertTo-WbRedacted;
  scripts/redact-evidence.ps1 applies the same rules to a file written by hand (for example cdb
  output). scripts/check-evidence.mjs (pnpm check:evidence) is the backstop in CI.

  Rules, in order:
  1. Environment blocks (a line "Environment: <address>" as cdb's !peb prints it, followed by
     indented NAME=value lines) are dropped entirely and replaced by one line saying so.
  2. $env:USERPROFILE (and any X:\Users\<USERNAME> path) becomes %USERPROFILE%.
  3. $env:USERNAME as a whole word becomes %USERNAME%.
  4. $env:COMPUTERNAME becomes the machine label (-MachineLabel, default %COMPUTERNAME%).
  5. NAME=value where NAME ends in KEY, TOKEN, SECRET or PASSWORD keeps the name and loses the value.

  ASCII only; runs under Windows PowerShell 5.1.
#>

Set-StrictMode -Version 2

function ConvertTo-WbRedacted {
  [CmdletBinding()]
  param(
    [Parameter(ValueFromPipeline = $true)] [AllowEmptyString()] [string[]]$Line,
    [string]$MachineLabel = '%COMPUTERNAME%',
    [string]$UserProfile = $env:USERPROFILE,
    [string]$UserName = $env:USERNAME,
    [string]$ComputerName = $env:COMPUTERNAME
  )
  begin {
    $all = New-Object System.Collections.Generic.List[string]
  }
  process {
    foreach ($l in $Line) { $all.Add($l) }
  }
  end {
    $inEnv = $false
    foreach ($l in $all) {
      if ($l -match '^\s*Environment:\s+[0-9a-fA-F`]+\s*$') {
        $inEnv = $true
        '    [environment block removed]'
        continue
      }
      if ($inEnv) {
        if ($l -match '^\s{4,}[^\s=][^=]*=') { continue }
        $inEnv = $false
      }
      $s = $l
      if ($UserProfile) { $s = $s -replace [regex]::Escape($UserProfile), '%USERPROFILE%' }
      if ($UserName) {
        $s = $s -replace ('[A-Za-z]:\\Users\\' + [regex]::Escape($UserName) + '(?=\\|\b)'), '%USERPROFILE%'
        $s = $s -replace ('\b' + [regex]::Escape($UserName) + '\b'), '%USERNAME%'
      }
      if ($ComputerName) { $s = $s -replace ('\b' + [regex]::Escape($ComputerName) + '\b'), $MachineLabel }
      $s = $s -replace '\b([A-Za-z0-9_]*(?:KEY|TOKEN|SECRET|PASSWORD))=\S*', '$1=<redacted>'
      $s
    }
  }
}

# Redact and write lines as UTF-8 (the evidence logs' encoding).
function Write-WbEvidence {
  param(
    [Parameter(Mandatory = $true)] [string]$Path,
    [Parameter(Mandatory = $true)] [AllowEmptyCollection()] [AllowEmptyString()] [string[]]$Lines,
    [string]$MachineLabel = '%COMPUTERNAME%'
  )
  $clean = @($Lines | ConvertTo-WbRedacted -MachineLabel $MachineLabel)
  $clean | Set-Content -Path $Path -Encoding utf8
}

Export-ModuleMember -Function ConvertTo-WbRedacted, Write-WbEvidence
