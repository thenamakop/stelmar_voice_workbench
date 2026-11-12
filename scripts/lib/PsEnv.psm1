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

# Makes numbers in text follow the invariant culture for the rest of this thread. PowerShell's -f
# operator uses the current culture, so on an en-IN machine 8776800 came out as 87,76,800 in the
# gate evidence. Call it once at the top of every gate script, after Set-WbCleanPSModulePath.
function Set-WbInvariantCulture {
  [System.Threading.Thread]::CurrentThread.CurrentCulture = [System.Globalization.CultureInfo]::InvariantCulture
  [System.Threading.Thread]::CurrentThread.CurrentUICulture = [System.Globalization.CultureInfo]::InvariantCulture
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

# With powershell -File, '-Gates a,b,c' arrives as the single string 'a,b,c'. Splits every item on
# commas and white space, so the list works with -File, -Command and an array alike.
function Split-WbList {
  param([string[]]$Items)
  @($Items | ForEach-Object { "$_" -split '[,\s]+' } | Where-Object { $_ })
}

Export-ModuleMember -Function Get-WbWindowsPowerShellModulePath, Set-WbCleanPSModulePath, Set-WbInvariantCulture, Invoke-WbWindowsPowerShell, Split-WbList
