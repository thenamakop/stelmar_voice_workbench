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

# Quotes one argument for a Windows command line (Start-Process joins its list with spaces and adds no quotes).
function ConvertTo-WbCommandLineArg {
  param([string]$Value)
  if ($Value -eq '') { return '""' }
  if ($Value -notmatch '[\s"]') { return $Value }
  '"' + ($Value -replace '(\\*)"', '$1$1\"' -replace '(\\+)$', '$1$1') + '"'
}

# Like Invoke-WbWindowsPowerShell, but the child's output goes to FILES, not to a pipe. A child that
# leaves orphan processes behind (a player or a capture that outlives a crashed gate) holds a pipe's
# write end open, so `& powershell ... | ForEach-Object` never returns; with files nothing is held, and
# this waits for the child process only (not for its descendants). Returns ExitCode, Output (stdout
# lines) and Error (stderr lines); the files are -OutFile and -OutFile + '.err'.
function Invoke-WbWindowsPowerShellToFile {
  param([Parameter(Mandatory = $true)] [string[]]$ArgumentList, [Parameter(Mandatory = $true)] [string]$OutFile)
  $exe = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
  $errFile = $OutFile + '.err'
  $saved = $env:PSModulePath
  try {
    $env:PSModulePath = Get-WbWindowsPowerShellModulePath
    $line = ($ArgumentList | ForEach-Object { ConvertTo-WbCommandLineArg ([string]$_) }) -join ' '
    $p = Start-Process -FilePath $exe -ArgumentList $line -PassThru -WindowStyle Hidden -RedirectStandardOutput $OutFile -RedirectStandardError $errFile
    $null = $p.Handle   # keep the handle, or ExitCode can be empty after the process ends
    $p.WaitForExit()
    $code = $p.ExitCode
  } finally {
    $env:PSModulePath = $saved
  }
  $o = $(if (Test-Path -LiteralPath $OutFile) { @(Get-Content -LiteralPath $OutFile) } else { @() })
  $e = $(if (Test-Path -LiteralPath $errFile) { @(Get-Content -LiteralPath $errFile) } else { @() })
  [pscustomobject]@{ ExitCode = $code; Output = @($o | ForEach-Object { "$_" }); Error = @($e | ForEach-Object { "$_" }) }
}
# With powershell -File, '-Gates a,b,c' arrives as the single string 'a,b,c'. Splits every item on
# commas and white space, so the list works with -File, -Command and an array alike.
function Split-WbList {
  param([string[]]$Items)
  @($Items | ForEach-Object { "$_" -split '[,\s]+' } | Where-Object { $_ })
}

Export-ModuleMember -Function Get-WbWindowsPowerShellModulePath, Set-WbCleanPSModulePath, Set-WbInvariantCulture, Invoke-WbWindowsPowerShell, Invoke-WbWindowsPowerShellToFile, ConvertTo-WbCommandLineArg, Split-WbList
