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

# Invoke-WbWindowsPowerShellToFile: the child's output goes to files, so an orphan the child leaves
# running cannot hold a pipe open and hang the caller (the 12 Nov 18:10 hashing gate).
Check 'ConvertTo-WbCommandLineArg leaves a plain word alone' ((ConvertTo-WbCommandLineArg 'abc') -eq 'abc')
Check 'ConvertTo-WbCommandLineArg quotes a path with spaces' ((ConvertTo-WbCommandLineArg 'C:\a b\c.ps1') -eq '"C:\a b\c.ps1"')
Check 'ConvertTo-WbCommandLineArg quotes an empty string' ((ConvertTo-WbCommandLineArg '') -eq '""')
Check 'ConvertTo-WbCommandLineArg escapes an inner quote' ((ConvertTo-WbCommandLineArg 'say "hi"') -eq '"say \"hi\""')
$tfOut = Join-Path ([IO.Path]::GetTempPath()) ('wb-tofile-' + [guid]::NewGuid().ToString('N') + '.out')
$orphanPid = $null
try {
  $r1 = Invoke-WbWindowsPowerShellToFile -ArgumentList @('-NoProfile', '-Command', '[Console]::Out.WriteLine("hello out"); [Console]::Error.WriteLine("hello err"); exit 5') -OutFile $tfOut
  Check 'the helper returns the exit code of the child' ($r1.ExitCode -eq 5)
  Check 'the helper returns stdout and stderr as text lines' (($r1.Output -join '|') -match 'hello out' -and ($r1.Error -join '|') -match 'hello err')
  Check 'an argument with spaces reaches the child as one argument' ((Invoke-WbWindowsPowerShellToFile -ArgumentList @('-NoProfile', '-Command', '[Console]::Out.WriteLine(''a   b'')') -OutFile $tfOut).Output[-1] -eq 'a   b')
  # A child that starts a long-running grandchild and exits at once: the caller must come back at once.
  $childCmd = '$p = Start-Process -FilePath "$env:SystemRoot\System32\WindowsPowerShell\v1.0\powershell.exe" -ArgumentList "-NoProfile","-Command","Start-Sleep -Seconds 120" -PassThru -WindowStyle Hidden; [Console]::Out.WriteLine("orphan=" + $p.Id)'
  $sw = [Diagnostics.Stopwatch]::StartNew()
  $r2 = Invoke-WbWindowsPowerShellToFile -ArgumentList @('-NoProfile', '-Command', $childCmd) -OutFile $tfOut
  $sw.Stop()
  $m = [regex]::Match(($r2.Output -join ' '), 'orphan=(\d+)')
  if ($m.Success) { $orphanPid = [int]$m.Groups[1].Value }
  Check 'a child that leaves an orphan behind returns at once (no pipe is held)' ($sw.Elapsed.TotalSeconds -lt 30 -and $r2.ExitCode -eq 0 -and $m.Success)
} finally {
  if ($orphanPid) { Stop-Process -Id $orphanPid -Force -ErrorAction SilentlyContinue }
  Remove-Item -LiteralPath $tfOut, ($tfOut + '.err') -ErrorAction SilentlyContinue
}

if ($fails) { Write-Host "$fails failed"; exit 1 }
Write-Host 'PsEnv tests passed'
