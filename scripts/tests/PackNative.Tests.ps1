# Tests for scripts/lib/PackNative.psm1: exit code, stdout, stderr and a missing program, with the
# caller running under $ErrorActionPreference = 'Stop'.
# Run: powershell -NoProfile -ExecutionPolicy Bypass -File scripts/tests/PackNative.Tests.ps1
# ASCII only; Windows PowerShell 5.1.
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
Import-Module (Join-Path $here '..\lib\PackNative.psm1') -Force
$fails = 0
function Check([string]$Name, [bool]$Ok) {
  if ($Ok) { Write-Host "ok   $Name" } else { Write-Host "FAIL $Name"; $script:fails++ }
}
$cmd = Join-Path $env:SystemRoot 'System32\cmd.exe'
$r = Invoke-PackNative -FilePath $cmd -ArgumentList @('/c', 'echo hello & exit 3')
Check 'the exit code and stdout come back' ($r.ExitCode -eq 3 -and ($r.Output -join '') -match 'hello')
$r = Invoke-PackNative -FilePath $cmd -ArgumentList @('/c', 'echo oops 1>&2 & exit 0')
Check 'stderr comes back as text and does not end the caller' ($r.ExitCode -eq 0 -and ($r.Error -join '') -match 'oops')
$r = Invoke-PackNative -FilePath (Join-Path $env:TEMP 'no-such-program-xyz.exe')
Check 'a missing program is exit -1 with a reason' ($r.ExitCode -eq -1 -and @($r.Error).Count -eq 1)
$r = Invoke-PackNative -FilePath $cmd -ArgumentList @('/c', 'ping -n 6 127.0.0.1 >nul') -TimeoutSeconds 1
Check 'a program that runs too long is stopped' ($r.ExitCode -eq -1 -and ($r.Error -join '') -match 'did not end')
$r = Invoke-PackNative -FilePath $cmd -ArgumentList @('/c', 'echo a b')
Check 'arguments with spaces survive' ($r.ExitCode -eq 0 -and ($r.Output -join '') -match 'a b')
if ($fails) { Write-Host "$fails failed"; exit 1 }
Write-Host 'PackNative tests passed'
