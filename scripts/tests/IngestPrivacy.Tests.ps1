# What the repository (public) may keep of a laptop's logs. The dry run of 17 Nov 2026 put the gateway address of the
# Wi-Fi network (10.20.30.1) into three evidence files, because the ingest removed only user paths and secret values.
# Now it also removes private addresses, MAC addresses, Wi-Fi names, serial numbers, a person's name in a device name
# and any string named with -Redact (or in an identity.json at the root of the zip), in the evidence, in the copy of
# summary.json and in the ledger rows. Public servers and Windows constants stay.
# Run: powershell -NoProfile -ExecutionPolicy Bypass -File scripts/tests/IngestPrivacy.Tests.ps1
# ASCII only; Windows PowerShell 5.1.
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo = (Resolve-Path (Join-Path $here '..\..')).Path
Import-Module (Join-Path $here '..\lib\Ingest.psm1') -Force
Import-Module (Join-Path $here '..\lib\PackCore.psm1') -Force
Import-Module (Join-Path $here '..\lib\PackNative.psm1') -Force
$fails = 0
function Check([string]$Name, [bool]$Ok) {
  if ($Ok) { Write-Host "ok   $Name" } else { Write-Host "FAIL $Name"; $script:fails++ }
}
function Redact-Line([string]$Line, [string[]]$Extra = @()) { [string](@(ConvertTo-IngestRedacted -Line @($Line) -Extra $Extra)[0]) }

# --- the rules ---
Check 'the gateway of the Wi-Fi network is removed' ((Redact-Line 'default route: 0.0.0.0/0 via 10.20.30.1 on Wi-Fi (ifIndex 6, metric 0)') -eq 'default route: 0.0.0.0/0 via <private-ip> on Wi-Fi (ifIndex 6, metric 0)')
Check '192.168.x.x and 172.16 to 172.31 and link-local addresses are removed' ((Redact-Line 'a 192.168.1.20 b 172.20.5.9 c 169.254.3.4') -eq 'a <private-ip> b <private-ip> c <private-ip>')
Check 'public servers stay (the probes of the network gate, a server the app contacted)' ((Redact-Line 'probe 1.1.1.1:443 8.8.8.8 150.171.27.11:443 172.32.0.1') -eq 'probe 1.1.1.1:443 8.8.8.8 150.171.27.11:443 172.32.0.1')
Check 'driver and build versions are not taken for addresses' ((Redact-Line 'driver 10.18.14.5067 and 31.0.101.7092, Windows 10.0.26200.7462, 10.0.1.2.3') -eq 'driver 10.18.14.5067 and 31.0.101.7092, Windows 10.0.26200.7462, 10.0.1.2.3')
Check 'a MAC address is removed' ((Redact-Line 'adapter 3C:52:82:AA:0B:19 and 3c-52-82-aa-0b-19') -eq 'adapter <mac> and <mac>')
Check 'a link-local IPv6 address is removed' ((Redact-Line 'fe80::1a2b:3c4d:5e6f:7a8b%6 up') -eq '<ipv6> up')
Check 'a Wi-Fi name is removed' ((Redact-Line '    SSID                   : My Home Network 5G') -eq '    SSID                   : <redacted>')
Check 'a serial number is removed' ((Redact-Line 'SerialNumber : PF3ABC12') -eq 'SerialNumber : <redacted>')
Check 'a person''s name in a device name is removed' ((Redact-Line 'Headphones (Anna''s AirPods Pro)') -eq 'Headphones (<name>''s AirPods Pro)')
Check 'an ordinary English possessive is not touched' ((Redact-Line 'the app''s window and Windows'' tray') -eq 'the app''s window and Windows'' tray')
Check 'user profile paths and secrets are still removed (plain and as written in JSON)' ((Redact-Line 'C:\Users\Anna\x\y') -eq '%USERPROFILE%\x\y' -and (Redact-Line '"C:\\Users\\Anna\\x"') -eq '"%USERPROFILE%\\x"' -and (Redact-Line 'HF_TOKEN=abc') -eq 'HF_TOKEN=<redacted>')
Check 'a power-plan GUID and the device ids of a virtual cable stay (Windows constants)' ((Redact-Line 'power plan: 381b4222-f694-41f0-9685-ff5bb260df2e  ROOT\MEDIA\0001') -eq 'power plan: 381b4222-f694-41f0-9685-ff5bb260df2e  ROOT\MEDIA\0001')
Check 'the computer name and user name given are removed, in any case, only as whole words' ((Redact-Line 'host JOHNS-LAPTOP user johnd. johndoe johnd' -Extra @('johns-laptop', 'JohnD')) -eq 'host <redacted> user <redacted>. johndoe <redacted>')
Check 'very short strings are never used as a pattern' ((Redact-Line 'a b ab' -Extra @('a', 'b', 'ab')) -eq 'a b ab')

# --- a whole ingest ---
$tmp = Join-Path ([IO.Path]::GetTempPath()) ('wb-ingpriv-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path (Join-Path $tmp 'scripts') | Out-Null
Copy-Item -LiteralPath (Join-Path $repo 'scripts\lib') -Destination (Join-Path $tmp 'scripts\lib') -Recurse
$gatesMd = Join-Path $tmp 'GATES.md'
[IO.File]::WriteAllText($gatesMd, (@('# Gate ledger', '', '## Gates', '', '| Phase | Gate item | Machine or profile | Status | Evidence | Date |', '| --- | --- | --- | --- | --- | --- |', '| 1 | Soak: 2 hours | Legion | PASS (PROVISIONAL) | x | 10 Nov 2026 |', '', '## Open items', '', '| Found in | Belongs to | Item | Status | Evidence | Date |', '| --- | --- | --- | --- | --- | --- |', '') -join "`r`n"), (New-Object Text.UTF8Encoding($false)))
$work = Join-Path $tmp 'work'
New-Item -ItemType Directory -Force -Path (Join-Path $work 'gates\network-off') | Out-Null
Set-Content -LiteralPath (Join-Path $work 'gates\network-off\gate.log') -Value @('default route via 10.20.30.1 on Wi-Fi', 'machine JOHNS-LAPTOP, user johnd, adapter 3C:52:82:AA:0B:19', 'probe 1.1.1.1:443') -Encoding UTF8
$g = [ordered]@{ id = 'network-off'; priority = 'P1'; status = 'PASS'; headline = 'no socket on JOHNS-LAPTOP via 192.168.1.1'; rig = 'n/a'; measured = [ordered]@{}; quick = $false; not_at_full_length = $false; ledger = @() }
$summary = [ordered]@{ pack_version = 1; source_commit = 'abc1234'; machine = 'Acme-Book-14-aabbcc'; role = 'low'; emulated = 'emu-low'; quick = $true; created = '2026-11-17T09:00:00Z'; gates = @($g); pending = @([ordered]@{ id = 'soak'; reason = 'waiting for JOHNS-LAPTOP'; ledger = @() }); not_in_pack = @(); still_changed = @() }
$zip = Join-Path $tmp 'run.zip'
New-PackZip -WorkDir $work -ZipPath $zip -Summary $summary -CommitText 'abc1234' | Out-Null
$ps = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
try {
  $r = Invoke-PackNative -FilePath $ps -ArgumentList @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', (Join-Path $repo 'scripts\ingest-gate-results.ps1'), '-Repo', $tmp, '-GatesMd', $gatesMd, '-Zip', $zip, '-Role', 'low', '-Redact', 'JOHNS-LAPTOP,johnd') -TimeoutSeconds 120
  if ($r.ExitCode -ne 0) { Write-Host (((@($r.Output) + @($r.Error)) -join "`n")) }
  Check 'the ingest ran' ($r.ExitCode -eq 0)
  $ev = Join-Path $tmp 'docs\evidence\gate-pack-dryrun\emu-low'
  $log = Get-Content -LiteralPath (Join-Path $ev 'gates\network-off\gate.log') -Raw
  Check 'the evidence log has no gateway, no computer name, no user name and no MAC address' ($log -notmatch '10\.20\.30\.1' -and $log -notmatch 'JOHNS' -and $log -notmatch 'johnd' -and $log -notmatch '3C:52' -and $log -match '<private-ip>' -and $log -match '<mac>')
  Check 'the public probe address stays in the evidence' ($log -match '1\.1\.1\.1:443')
  $sj = Get-Content -LiteralPath (Join-Path $ev 'summary.json') -Raw
  Check 'the copy of summary.json is cleaned too' ($sj -notmatch 'JOHNS' -and $sj -notmatch '192\.168\.1\.1' -and $sj -match '<private-ip>')
  Check 'RESULTS.txt is cleaned' ((Get-Content -LiteralPath (Join-Path $ev 'RESULTS.txt') -Raw) -notmatch 'JOHNS')
  $md = Get-Content -LiteralPath $gatesMd -Raw
  Check 'the ledger rows are cleaned' ($md -notmatch 'JOHNS' -and $md -notmatch '192\.168\.1\.1' -and $md -match 'network-off')
} finally {
  Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue
}

if ($fails -gt 0) { Write-Host "$fails failed"; exit 1 }
Write-Host 'IngestPrivacy tests passed'
