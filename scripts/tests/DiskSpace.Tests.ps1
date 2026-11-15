# The recorder gates need free disk space: the gate-build app refuses to start a session under 5 GB free
# on the recording volume. Two microphone runs ended "the app ended before recording started" because D: was
# nearly full and the check allowed 2 GB. The gates now check first (PENDING, not INVALID) and show the app's own error.
# Run: powershell -NoProfile -ExecutionPolicy Bypass -File scripts/tests/DiskSpace.Tests.ps1
# ASCII only; Windows PowerShell 5.1.
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$root = (Resolve-Path (Join-Path $here '..')).Path
$fails = 0
function Check([string]$Name, [bool]$Ok) {
  if ($Ok) { Write-Host "ok   $Name" } else { Write-Host "FAIL $Name"; $script:fails++ }
}
Import-Module (Join-Path $root 'lib\PackGate.psm1') -Force

# The need is never under the app's own 5 GB, and grows with the run.
Check 'the disk need is above the 5 GB the app demands' ((Get-PackDiskNeedGB -Minutes 0) -gt 5)
Check 'a 2 h run needs more than a 15 minute run' ((Get-PackDiskNeedGB -Minutes 120) -gt (Get-PackDiskNeedGB -Minutes 15))

# Not enough: throws a message that is PENDING, with the amounts.
$msg = ''
try { Assert-PackFreeDisk -Path 'D:\x\y' -NeedGB 6.0 -What 'the test' -FreeBytes (2.4 * 1GB) } catch { $msg = $_.Exception.Message }
Check 'low disk space throws' ($msg -ne '')
Check 'the message gives the free and the needed amounts' ($msg -match 'only 2[.,]4 GB free on D:' -and $msg -match '6[.,]0 GB needed')
Check 'low disk space is PENDING, not INVALID' ((Get-PackErrorStatus $msg) -eq 'PENDING')

# Enough: no throw.
$ok = $true
try { Assert-PackFreeDisk -Path 'D:\x\y' -NeedGB 6.0 -FreeBytes (50 * 1GB) } catch { $ok = $false }
Check 'enough disk space does not throw' $ok

# The app's own error is read from its stop report.
$tmp = Join-Path ([IO.Path]::GetTempPath()) ('wb-disk-test-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $tmp | Out-Null
try {
  $rp = Join-Path $tmp 'stop-report.json'
  [IO.File]::WriteAllText($rp, '{"error": "Only 2.6 GB free on D:/x; at least 5 GB is needed to start a session"}', (New-Object Text.UTF8Encoding($false)))
  Check 'the app error text is read from stop-report.json' ((Get-PackAppReportError $rp) -match 'at least 5 GB is needed')
  Check 'a missing report gives an empty string' ((Get-PackAppReportError (Join-Path $tmp 'none.json')) -eq '')
  [IO.File]::WriteAllText($rp, '{"ok": true}', (New-Object Text.UTF8Encoding($false)))
  Check 'a report without an error gives an empty string' ((Get-PackAppReportError $rp) -eq '')
} finally { Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue }

# The gates use them.
$mic = Get-Content -LiteralPath (Join-Path $root 'gate-pack\gates\mic-run.ps1') -Raw
Check 'mic-run checks the disk before it plays anything' ($mic -match 'Assert-PackFreeDisk' -and $mic.IndexOf('Assert-PackFreeDisk') -lt $mic.IndexOf('Request-PackPrompt'))
Check 'mic-run shows what the app said when it ends early' ($mic -match 'Get-PackAppReportError')
$rg = Get-Content -LiteralPath (Join-Path $root 'lib\RecorderGates.psm1') -Raw
Check 'every pack recorder gate checks the disk' ($rg -match 'Assert-PackFreeDisk')
$leg = Get-Content -LiteralPath (Join-Path $root 'gates\recorder-gate.ps1') -Raw
Check 'recorder-gate shows what the app said when it ends early' ($leg -match 'The app said')

if ($fails -gt 0) { Write-Host "$fails failed"; exit 1 }
Write-Host 'all passed'
