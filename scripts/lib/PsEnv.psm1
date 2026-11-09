<#
  PsEnv.psm1: keeps Windows PowerShell 5.1 working when it is started from another shell.

  A parent shell (PowerShell 7, or the Bash tool that carries a PowerShell 7 module path) hands
  its PSModulePath to the child. Windows PowerShell 5.1 then finds PowerShell 7's copy of
  Microsoft.PowerShell.Utility first and fails with "The term 'Get-FileHash' is not recognized".
  The fix is to give 5.1 its own module path: Invoke-WbWindowsPowerShell does that for a child it
  starts, and Set-WbCleanPSModulePath does it inside a script that is already running in 5.1.
  ASCII only.
#>

# The module path a fresh Windows PowerShell 5.1 session has.
function Get-WbWindowsPowerShellModulePath {
  $docs = [Environment]::GetFolderPath('MyDocuments')
  $pf = $env:ProgramFiles
  $sys = Join-Path $env:SystemRoot 'System32'
  @(
    (Join-Path $docs 'WindowsPowerShell\Modules'),
    (Join-Path $pf 'WindowsPowerShell\Modules'),
    (Join-Path $sys 'WindowsPowerShell\v1.0\Modules')
  ) -join ';'
}

# In a running Windows PowerShell 5.1 session, replace an inherited PowerShell 7 module path.
# Does nothing in PowerShell 7.
function Set-WbCleanPSModulePath {
  if ($PSVersionTable.PSEdition -eq 'Desktop') { $env:PSModulePath = Get-WbWindowsPowerShellModulePath }
}

# Runs powershell.exe (Windows PowerShell 5.1) with the clean module path and returns its output
# lines (stdout and stderr as text); $LASTEXITCODE is the child's exit code afterwards. The
# caller's PSModulePath is restored.
function Invoke-WbWindowsPowerShell {
  param([Parameter(Mandatory = $true)] [string[]]$ArgumentList)
  $exe = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
  $saved = $env:PSModulePath
  $prevPref = $ErrorActionPreference
  try {
    $env:PSModulePath = Get-WbWindowsPowerShellModulePath
    # A child's stderr merged with 2>&1 becomes error records; collect them as text instead.
    $ErrorActionPreference = 'Continue'
    $out = @(& $exe @ArgumentList 2>&1 | ForEach-Object { "$_" })
  } finally {
    $ErrorActionPreference = $prevPref
    $env:PSModulePath = $saved
  }
  $out
}

Export-ModuleMember -Function Get-WbWindowsPowerShellModulePath, Set-WbCleanPSModulePath, Invoke-WbWindowsPowerShell
