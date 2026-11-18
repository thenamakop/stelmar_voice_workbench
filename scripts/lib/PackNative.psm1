<#
  PackNative.psm1: run a native program and get its exit code and output without redirecting stderr in
  PowerShell (under Windows PowerShell 5.1 a redirected stderr line becomes an error record and ends a
  script that runs with $ErrorActionPreference = 'Stop'; see Invoke-WbNative in GateRun.psm1). This one
  starts the program with System.Diagnostics.Process and reads both streams itself, so the pack's
  modules do not need GateRun.psm1.

  Windows PowerShell 5.1, ASCII only.
#>

function ConvertTo-PackNativeArg {
  param([string]$Text)
  if ($null -eq $Text -or $Text -eq '') { return '""' }
  if ($Text -notmatch '[\s"]') { return $Text }
  '"' + ($Text -replace '(\\*)"', '$1$1\"' -replace '(\\+)$', '$1$1') + '"'
}

# Returns ExitCode (-1 if the program cannot start or does not end in TimeoutSeconds), Output and Error
# as arrays of lines.
function Invoke-PackNative {
  [CmdletBinding()]
  param(
    [Parameter(Mandatory = $true)] [string]$FilePath,
    [string[]]$ArgumentList = @(),
    [int]$TimeoutSeconds = 120
  )
  $psi = New-Object System.Diagnostics.ProcessStartInfo
  $psi.FileName = $FilePath
  $psi.Arguments = (@($ArgumentList | ForEach-Object { ConvertTo-PackNativeArg ([string]$_) }) -join ' ')
  $psi.UseShellExecute = $false
  $psi.CreateNoWindow = $true
  $psi.RedirectStandardOutput = $true
  $psi.RedirectStandardError = $true
  $p = $null
  try { $p = [System.Diagnostics.Process]::Start($psi) }
  catch { return [pscustomobject]@{ ExitCode = -1; Output = @(); Error = @("$($_.Exception.Message)") } }
  $outTask = $p.StandardOutput.ReadToEndAsync()
  $errTask = $p.StandardError.ReadToEndAsync()
  if (-not $p.WaitForExit($TimeoutSeconds * 1000)) {
    try { $p.Kill() } catch { }
    return [pscustomobject]@{ ExitCode = -1; Output = @(); Error = @("did not end in $TimeoutSeconds s: $FilePath") }
  }
  $p.WaitForExit()
  $o = $outTask.Result
  $e = $errTask.Result
  $code = $p.ExitCode
  $p.Dispose()
  [pscustomobject]@{
    ExitCode = $code
    Output = @(($o -split "\r?\n") | Where-Object { $_ -ne '' })
    Error = @(($e -split "\r?\n") | Where-Object { $_ -ne '' })
  }
}

Export-ModuleMember -Function Invoke-PackNative, ConvertTo-PackNativeArg
