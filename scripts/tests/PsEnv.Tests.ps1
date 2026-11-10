# Tests for scripts/lib/PsEnv.psm1: Windows PowerShell 5.1 started from a shell that carries a
# PowerShell 7 module path (the Bash tool, pwsh) must still find Get-FileHash.
# Run: powershell -NoProfile -ExecutionPolicy Bypass -File scripts/tests/PsEnv.Tests.ps1
# ASCII only; Windows PowerShell 5.1.
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
Import-Module (Join-Path $here '..\lib\PsEnv.psm1') -Force
$fails = 0
function Check([string]$Name, [bool]$Ok) {
  if ($Ok) { Write-Host "ok   $Name" } else { Write-Host "FAIL $Name"; $script:fails++ }
}

$ps7Path = 'C:\Users\someone\Documents\PowerShell\Modules;C:\Program Files\PowerShell\Modules;c:\program files\powershell\7\Modules;C:\Program Files\WindowsPowerShell\Modules;C:\WINDOWS\system32\WindowsPowerShell\v1.0\Modules'
$probe = '[Console]::Out.WriteLine(((Get-Command Get-FileHash -ErrorAction SilentlyContinue) -ne $null)); [Console]::Out.WriteLine($env:PSModulePath)'
$saved = $env:PSModulePath
try {
  # Control: the bad inherited path breaks 5.1 on a machine that has PowerShell 7 installed
  # (reported, not required, so the test also passes on a machine without PowerShell 7).
  $env:PSModulePath = $ps7Path
  $raw = @(& powershell.exe -NoProfile -Command $probe 2>&1 | ForEach-Object { "$_" })
  Write-Host ("info raw child with a PowerShell 7 module path finds Get-FileHash: " + $raw[0])

  # The helper gives the child its own module path whatever the parent carries.
  $out = @(Invoke-WbWindowsPowerShell -ArgumentList @('-NoProfile', '-Command', $probe))
  Check 'a child started with the helper finds Get-FileHash' ($out[0] -eq 'True')
  Check 'the child path has no PowerShell 7 entry' (($out[1] -notmatch 'PowerShell\\7') -and ($out[1] -notmatch '\\Documents\\PowerShell\\'))
  Check 'the child path has the 5.1 system modules' ($out[1] -match 'v1\.0\\Modules')
  Check 'the caller path is restored' ($env:PSModulePath -eq $ps7Path)

  # A script already running in 5.1 with a bad path repairs itself.
  $self = (Resolve-Path (Join-Path $here '..\lib\PsEnv.psm1')).Path
  $inner = "Import-Module '$self' -Force; Set-WbCleanPSModulePath; [Console]::Out.WriteLine(((Get-Command Get-FileHash -ErrorAction SilentlyContinue) -ne `$null))"
  $env:PSModulePath = $ps7Path
  $r = @(& powershell.exe -NoProfile -Command $inner 2>&1 | ForEach-Object { "$_" })
  Check 'Set-WbCleanPSModulePath repairs a running 5.1 session' ($r[-1] -eq 'True')

  # The exit code of the child is visible.
  $null = Invoke-WbWindowsPowerShell -ArgumentList @('-NoProfile', '-Command', 'exit 7')
  Check 'the child exit code is kept' ($LASTEXITCODE -eq 7)
} finally {
  $env:PSModulePath = $saved
}

# Every launcher goes through the helper or repairs its own module path, and no script starts
# powershell.exe bare.
$root = (Resolve-Path (Join-Path $here '..')).Path
$bare = @(Get-ChildItem -Path $root -Recurse -Include *.ps1 -File |
  Where-Object { $_.FullName -notmatch '\\tests\\PsEnv' } |
  Where-Object { (Get-Content -LiteralPath $_.FullName -Raw) -match '&\s*powershell\.exe' })
Check ('no script calls powershell.exe bare: ' + (($bare | ForEach-Object { $_.Name }) -join ', ')) ($bare.Count -eq 0)
$launchers = @('gates\recorder-gate.ps1', 'gates\kill-gate.ps1', 'gates\emulator-check.ps1', 'gates\run-phase1-gates.ps1',
  'offline-check.ps1', 'offline-session-check.ps1', 'tests\run-tests.ps1')
foreach ($name in $launchers) {
  $t = Get-Content -LiteralPath (Join-Path $root $name) -Raw
  Check "$name repairs or sets the module path" ($t -match 'Set-WbCleanPSModulePath|Invoke-WbWindowsPowerShell')
}

if ($fails) { Write-Host "$fails failed"; exit 1 }
Write-Host 'PsEnv tests passed'
