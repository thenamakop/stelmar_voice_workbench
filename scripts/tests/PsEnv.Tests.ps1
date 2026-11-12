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
  'gates\rig-selftest.ps1', 'offline-check.ps1', 'offline-session-check.ps1', 'tests\run-tests.ps1')
foreach ($name in $launchers) {
  $t = Get-Content -LiteralPath (Join-Path $root $name) -Raw
  Check "$name repairs or sets the module path" ($t -match 'Set-WbCleanPSModulePath|Invoke-WbWindowsPowerShell')
}

# Numbers in evidence follow the invariant culture: no 87,76,800 on an en-IN machine.
$gateScripts = @('gates\recorder-gate.ps1', 'gates\kill-gate.ps1', 'gates\emulator-check.ps1', 'gates\run-phase1-gates.ps1',
  'gates\rescore-run.ps1', 'gates\rig-selftest.ps1', 'offline-check.ps1', 'offline-session-check.ps1')
foreach ($name in $gateScripts) {
  $t = Get-Content -LiteralPath (Join-Path $root $name) -Raw
  Check "$name sets the invariant culture" ($t -match 'Set-WbInvariantCulture')
}
$savedCulture = [System.Threading.Thread]::CurrentThread.CurrentCulture
try {
  [System.Threading.Thread]::CurrentThread.CurrentCulture = [System.Globalization.CultureInfo]::GetCultureInfo('en-IN')
  Check 'control: under en-IN the -f operator groups 8776800 as 87,76,800' ((('{0:n0}' -f 8776800) -eq '87,76,800'))
  Set-WbInvariantCulture
  Check 'after Set-WbInvariantCulture 8776800 is 8,776,800' ((('{0:n0}' -f 8776800) -eq '8,776,800'))
  Check 'after Set-WbInvariantCulture a decimal point is a point' ((('{0:n1}' -f 1234.5) -eq '1,234.5'))
} finally {
  [System.Threading.Thread]::CurrentThread.CurrentCulture = $savedCulture
}

# Split-WbList: a comma list given with -File arrives as one string.
Check 'Split-WbList splits one comma string' (((Split-WbList @('hashing,panic,webview,kill,load,soak')) -join '|') -eq 'hashing|panic|webview|kill|load|soak')
Check 'Split-WbList keeps an array as it is' (((Split-WbList @('a', 'b')) -join '|') -eq 'a|b')
Check 'Split-WbList drops empty items and spaces' (((Split-WbList @('a, b,,c', '')) -join '|') -eq 'a|b|c')
$probeScript = Join-Path ([IO.Path]::GetTempPath()) ('wb-split-probe-' + [guid]::NewGuid().ToString('N') + '.ps1')
try {
  Set-Content -LiteralPath $probeScript -Encoding ascii -Value @(
    'param([string[]]$Gates = @("x"))',
    ('Import-Module "' + (Join-Path $here '..\lib\PsEnv.psm1') + '" -Force'),
    '$Gates = Split-WbList $Gates',
    '[Console]::Out.WriteLine(($Gates.Count.ToString() + ":" + ($Gates -join "|")))')
  $o = @(Invoke-WbWindowsPowerShell -ArgumentList @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $probeScript, '-Gates', 'hashing,panic,webview'))
  Check 'a real powershell -File call with -Gates a,b,c gives three gates' ($o[-1] -eq '3:hashing|panic|webview')
} finally { Remove-Item -LiteralPath $probeScript -ErrorAction SilentlyContinue }
$rg = Get-Content -LiteralPath (Join-Path $here '..\gates\run-phase1-gates.ps1') -Raw
Check 'run-phase1-gates.ps1 splits the gate list with Split-WbList' ($rg -match 'Split-WbList \$Gates')

if ($fails) { Write-Host "$fails failed"; exit 1 }
Write-Host 'PsEnv tests passed'
