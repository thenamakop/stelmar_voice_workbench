# Tests for scripts/lib/Redact.psm1 (ConvertTo-WbRedacted, with fixed names so they run anywhere).
# Run: powershell -NoProfile -ExecutionPolicy Bypass -File scripts/tests/Redact.Tests.ps1
# No Pester needed. Exit code 1 when any case fails. ASCII only; Windows PowerShell 5.1.
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
Import-Module (Join-Path $here '..\lib\Redact.psm1') -Force

$script:failures = 0
$script:passes = 0
function Assert-Equal($Expected, $Actual, [string]$What) {
  if ($Expected -ne $Actual) { throw "$What : expected '$Expected', got '$Actual'" }
}
function Test-Case([string]$Name, [scriptblock]$Body) {
  try { & $Body; $script:passes++; Write-Host "PASS  $Name" }
  catch { $script:failures++; Write-Host "FAIL  $Name : $($_.Exception.Message)" }
}
function Redact([string[]]$Lines, [string]$Label = '%COMPUTERNAME%') {
  @($Lines | ConvertTo-WbRedacted -UserProfile 'C:\Users\jdoe' -UserName 'jdoe' -ComputerName 'DESKTOP-AB12CD' -MachineLabel $Label)
}

Test-Case 'a) cdb environment block is dropped, the lines around it stay' {
  $out = Redact @(
    "    DllPath:      '< Name not readable >'",
    '    Environment:  0000023439e1d2e0',
    '        ALLUSERSPROFILE=C:\ProgramData',
    '        SOME_TOOL_MESSAGING_TOKEN=abc123',
    '        USERPROFILE=C:\Users\jdoe',
    '        windir=C:\WINDOWS',
    'quit:'
  )
  Assert-Equal 3 $out.Count 'line count'
  Assert-Equal '    [environment block removed]' $out[1] 'marker'
  Assert-Equal 'quit:' $out[2] 'line after the block'
}

Test-Case 'b) user profile path becomes %USERPROFILE%, in any case' {
  $out = Redact @('Loading Dump File [c:\users\JDOE\AppData\Local\x.dmp]', '--user-data-dir="C:\Users\jdoe\AppData"')
  Assert-Equal 'Loading Dump File [%USERPROFILE%\AppData\Local\x.dmp]' $out[0] 'lower case'
  Assert-Equal '--user-data-dir="%USERPROFILE%\AppData"' $out[1] 'command line'
}

Test-Case 'c) user name and computer name are replaced as whole words only' {
  $out = Redact @('Machine: DESKTOP-AB12CD   user jdoe', 'jdoes and xjdoe stay') 'Legion'
  Assert-Equal 'Machine: Legion   user %USERNAME%' $out[0] 'names'
  Assert-Equal 'jdoes and xjdoe stay' $out[1] 'partial words'
}

Test-Case 'd) KEY, TOKEN, SECRET and PASSWORD values are removed, names kept' {
  $out = Redact @('GITHUB_TOKEN=ghp_x API_KEY=1 DB_PASSWORD=p', 'MY_SECRET=s and --disable-features=Foo')
  Assert-Equal 'GITHUB_TOKEN=<redacted> API_KEY=<redacted> DB_PASSWORD=<redacted>' $out[0] 'values'
  Assert-Equal 'MY_SECRET=<redacted> and --disable-features=Foo' $out[1] 'other options untouched'
}

Test-Case 'e) an indented NAME=value line outside an environment block is kept' {
  $out = Redact @('  probe: 1.1.1.1:443', '        mode=clean')
  Assert-Equal 2 $out.Count 'line count'
  Assert-Equal '        mode=clean' $out[1] 'kept'
}

Write-Host ""
Write-Host "Redact tests: $script:passes passed, $script:failures failed"
if ($script:failures) { exit 1 }
