# The external SSD of the gate pack is the mirror's drive. Before 17 Nov 2026 a typed drive letter was never
# checked (a missing SSD made the mirror check, and so the recorder gate, FAIL), the evidence text always said
# "separate drive pending", and the ingest closed the mirror row on a mirror that was on the recordings' own drive.
# Run: powershell -NoProfile -ExecutionPolicy Bypass -File scripts/tests/BackupDrive.Tests.ps1
# ASCII only; Windows PowerShell 5.1.
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$root = (Resolve-Path (Join-Path $here '..')).Path
$fails = 0
function Check([string]$Name, [bool]$Ok) {
  if ($Ok) { Write-Host "ok   $Name" } else { Write-Host "FAIL $Name"; $script:fails++ }
}
Import-Module (Join-Path $root 'lib\PackGate.psm1') -Force
Import-Module (Join-Path $root 'lib\PackCore.psm1') -Force
Import-Module (Join-Path $root 'lib\Ingest.psm1') -Force

# --- the answer to "type the drive letter of the external SSD" ---
$yes = { param($l) $true }
$no = { param($l) $false }
$a = Resolve-PackBackupAnswer -Answer '' -DriveReady $yes
Check 'Enter means C: and says the copy stays on the same drive' ($a.Root -eq 'C:\STELMAR-Gate-Backup' -and -not $a.Ask -and $a.Message -match 'same drive')
$a = Resolve-PackBackupAnswer -Answer 'e:' -DriveReady $yes
Check 'a letter whose drive is there is taken, in capitals' ($a.Root -eq 'E:\STELMAR-Gate-Backup' -and -not $a.Ask)
$a = Resolve-PackBackupAnswer -Answer 'E:\' -DriveReady $yes
Check 'E:\ is read as the drive E' ($a.Root -eq 'E:\STELMAR-Gate-Backup')
$a = Resolve-PackBackupAnswer -Answer 'F' -DriveReady $no
Check 'a letter whose drive is not there asks again and says so' ($a.Ask -and $a.Root -eq '' -and $a.Message -match 'F: is not there')
$a = Resolve-PackBackupAnswer -Answer 'my ssd' -DriveReady $yes
Check 'text that is not a drive letter asks again' ($a.Ask -and $a.Message -match 'drive letter')

# --- the drive is checked before a gate records ---
$threw = ''
try { Assert-PackBackupDrive -BackupRoot 'E:\STELMAR-Gate-Backup' -DriveReady $false } catch { $threw = $_.Exception.Message }
Check 'a missing backup drive stops the gate with a message that names the drive and the way out' ($threw -match '^The backup drive E: is not connected' -and $threw -match 'BackupRoot C:')
Check 'that message is PENDING (waiting), never FAIL or INVALID' ((Get-PackErrorStatus -Message $threw) -eq 'PENDING')
$ok = $true
try { Assert-PackBackupDrive -BackupRoot 'E:\STELMAR-Gate-Backup' -DriveReady $true } catch { $ok = $false }
Check 'a backup drive that is there passes' $ok
$ok = $true
try { Assert-PackBackupDrive -BackupRoot '' } catch { $ok = $false }
Check 'no backup folder at all passes (the app then has no mirror)' $ok
$ok = $true
try { Assert-PackBackupDrive -BackupRoot 'C:\STELMAR-Gate-Backup' } catch { $ok = $false }
Check 'C: is always there' $ok

# --- same drive or not ---
Check 'recordings and mirror on C: are the same drive' (Test-PackMirrorSameDrive -WorkPath 'C:\STELMAR-Gate\pack\work' -BackupPath 'C:\STELMAR-Gate-Backup')
Check 'the letter is compared without regard to case' (Test-PackMirrorSameDrive -WorkPath 'c:\a' -BackupPath 'C:\b')
Check 'recordings on C: and mirror on E: are not the same drive' (-not (Test-PackMirrorSameDrive -WorkPath 'C:\a' -BackupPath 'E:\b'))
Check 'an unknown drive counts as the same (never claims a separate copy)' (Test-PackMirrorSameDrive -WorkPath '\\server\share' -BackupPath 'E:\b')

# --- the ingest keeps the mirror row open when the mirror was on the recordings' own drive ---
$fixture = @(
  '## Machines', '', '| Name | Make and model | CPU | RAM | GPUs and driver versions | First used |', '| --- | --- | --- | --- | --- | --- |', '',
  '## Gates', '', '| Phase | Gate item | Machine or profile | Status | Evidence | Date |', '| --- | --- | --- | --- | --- | --- |',
  '| 1 | Mirror SHA-256 equals the originals after Stop | low-baseline and mid laptops | PROVISIONAL (mirror on the same machine, separate drive pending) | docs/x | 10 Nov 2026 |', '',
  '## Open items', '', '| Found in | Belongs to | Item | Status | Evidence | Date |', '| --- | --- | --- | --- | --- | --- |')
function Summary([bool]$Same) {
  $o = [ordered]@{
    machine = 'M1'; created = '2026-11-18T10:00:00Z'; quick = $false
    gates = @([ordered]@{ id = 'soak'; status = 'PASS'; headline = 'ok'; rig = 'hardware interface loopback'; quick = $false; measured = [ordered]@{ rig = 'loopback'; mirror_same_drive = $Same }
        ledger = @([ordered]@{ table = 'gates'; match = 'Mirror SHA-256 equals the originals after Stop'; kind = 'verdict'; needs_separate_mirror_drive = $true }) })
    pending = @(); not_in_pack = @()
  }
  ($o | ConvertTo-Json -Depth 8) | ConvertFrom-Json
}
$u = Update-IngestGatesLines -Lines $fixture -Summary (Summary $true) -Role low -Date '18 Nov 2026'
$row = @($u.Lines | Where-Object { $_ -match 'Mirror SHA-256' })[0]
Check 'a mirror on the recordings'' own drive is recorded as a stand-in result' ($row -match 'low/soak FALLBACK')
Check 'and the row keeps its old status (the separate drive is still pending)' ($row -match 'PROVISIONAL \(mirror on the same machine, separate drive pending\)' -and $row -notmatch 'PASS \(gate pack\)')
$u2 = Update-IngestGatesLines -Lines $u.Lines -Summary (Summary $true) -Role mid -Date '19 Nov 2026'
$row = @($u2.Lines | Where-Object { $_ -match 'Mirror SHA-256' })[0]
Check 'even when both laptops ran with a mirror on their own drive the row stays open' ($row -notmatch 'PASS \(gate pack\)' -and $row -match 'mid/soak FALLBACK')
$s3 = Summary $false
$u3 = Update-IngestGatesLines -Lines $fixture -Summary $s3 -Role low -Date '18 Nov 2026'
$u3 = Update-IngestGatesLines -Lines $u3.Lines -Summary $s3 -Role mid -Date '19 Nov 2026'
$row = @($u3.Lines | Where-Object { $_ -match 'Mirror SHA-256' })[0]
Check 'with a mirror on a separate drive on both laptops the row closes as before' ($row -match 'PASS \(gate pack\)')

# --- the wiring ---
$m = Get-Content -LiteralPath (Join-Path $root 'gate-pack\gates.manifest.json') -Raw
Check 'the soak entry marks the mirror row as needing a separate drive' ($m -match 'Mirror SHA-256 equals the originals after Stop", "kind": "verdict", "needs_separate_mirror_drive": true')
$rg = Get-Content -LiteralPath (Join-Path $root 'lib\RecorderGates.psm1') -Raw
Check 'the recorder gates check the backup drive first and record where the mirror was' ($rg -match 'Assert-PackBackupDrive' -and $rg -match 'mirror_same_drive')
$mr = Get-Content -LiteralPath (Join-Path $root 'gate-pack\gates\mic-run.ps1') -Raw
Check 'the microphone run checks the backup drive first' ($mr -match 'Assert-PackBackupDrive')
$leg = Get-Content -LiteralPath (Join-Path $root 'gates\recorder-gate.ps1') -Raw
Check 'the evidence line no longer says "separate drive pending" when the mirror is on another drive' ($leg -notmatch 'instead of an external SSD' -and $leg -match 'a different drive letter')
$rn = Get-Content -LiteralPath (Join-Path $root 'gate-pack\run-gates.ps1') -Raw
Check 'the runner asks again for a drive letter that is not there' ($rn -match 'Resolve-PackBackupAnswer')
$rd = Get-Content -LiteralPath (Join-Path $root 'gate-pack\README.txt') -Raw
Check 'the README says the SSD is recommended, not required, and what happens without one' ($rd -match 'recommended, not required' -and $rd -match 'press\s+Enter' -and $rd -match 'waiting')

if ($fails -gt 0) { Write-Host "$fails failed"; exit 1 }
Write-Host 'BackupDrive tests passed'
