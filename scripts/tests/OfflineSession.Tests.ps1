# Tests for scripts/lib/OfflineSession.psm1: private-address masking and the offline verdict.
# Run: powershell -NoProfile -ExecutionPolicy Bypass -File scripts/tests/OfflineSession.Tests.ps1
# No Pester needed. Exit code 1 when any case fails. ASCII only; Windows PowerShell 5.1.
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
Import-Module (Join-Path $here '..\lib\PsEnv.psm1') -Force
Import-Module (Join-Path $here '..\lib\OfflineSession.psm1') -Force
$script:failures = 0
function Test-Case([string]$Name, [scriptblock]$Body) {
  try { & $Body; Write-Host "PASS  $Name" } catch { $script:failures++; Write-Host "FAIL  $Name : $($_.Exception.Message)" }
}
function Assert-Equal($Actual, $Expected) {
  if ($Actual -ne $Expected) { throw "expected [$Expected], got [$Actual]" }
}
$M = '<private-ip>'

Test-Case 'every private IPv4 range is masked, at both ends' {
  foreach ($a in '10.0.0.1', '10.255.255.254', '172.16.0.1', '172.31.255.255', '192.168.0.1', '192.168.255.255', '169.254.0.1', '169.254.10.20') {
    Assert-Equal (ConvertTo-WbPrivateMasked -Text "peer $($a):443 Established") "peer $($M):443 Established"
  }
}
Test-Case 'addresses just outside the private ranges are left alone' {
  foreach ($a in '172.15.255.255', '172.32.0.1', '192.167.1.1', '192.169.1.1', '11.0.0.1', '169.253.1.1', '169.255.1.1', '9.255.255.255') {
    Assert-Equal (ConvertTo-WbPrivateMasked -Text "peer $a") "peer $a"
  }
}
Test-Case 'a public address is left unmasked' {
  Assert-Equal (ConvertTo-WbPrivateMasked -Text 'TCP 142.250.77.14:443 Established') 'TCP 142.250.77.14:443 Established'
  Assert-Equal (ConvertTo-WbPrivateMasked -Text 'TCP 8.8.8.8:53 Established') 'TCP 8.8.8.8:53 Established'
}
Test-Case 'loopback and unspecified addresses are left alone' {
  foreach ($a in '127.0.0.1', '0.0.0.0', '::1', '::') {
    Assert-Equal (ConvertTo-WbPrivateMasked -Text "bound $a") "bound $a"
  }
}
Test-Case 'IPv6 unique local (fc00::/7) is masked, including fd00::' {
  Assert-Equal (ConvertTo-WbPrivateMasked -Text 'peer [fc00::1]:443') "peer [$M]:443"
  Assert-Equal (ConvertTo-WbPrivateMasked -Text 'peer [fd12:3456:789a::1]:443') "peer [$M]:443"
  Assert-Equal (ConvertTo-WbPrivateMasked -Text 'peer [fdff:ffff:ffff:ffff:ffff:ffff:ffff:ffff]:80') "peer [$M]:80"
}
Test-Case 'IPv6 link local (fe80::/10) is masked, with and without a zone id' {
  Assert-Equal (ConvertTo-WbPrivateMasked -Text 'peer [fe80::1]:443') "peer [$M]:443"
  Assert-Equal (ConvertTo-WbPrivateMasked -Text 'peer [fe80::a1b2:c3d4:e5f6:1234%12]:443') "peer [$M]:443"
  Assert-Equal (ConvertTo-WbPrivateMasked -Text 'listen fe80::1%7') "listen $M"
  Assert-Equal (ConvertTo-WbPrivateMasked -Text 'peer [febf::1]:1') "peer [$M]:1"
}
Test-Case 'IPv6 just outside the private ranges and public IPv6 are left alone' {
  foreach ($a in 'fec0::1', 'fe7f::1', 'fb00::1', 'fe00::1', '2607:f8b0:4005:80a::200e', '2001:4860:4860::8888') {
    Assert-Equal (ConvertTo-WbPrivateMasked -Text "peer [$a]:443") "peer [$a]:443"
  }
}
Test-Case 'an IPv4-mapped IPv6 address is masked when the IPv4 part is private only' {
  Assert-Equal (ConvertTo-WbPrivateMasked -Text 'peer ::ffff:192.168.1.5') "peer $M"
  Assert-Equal (ConvertTo-WbPrivateMasked -Text 'peer ::ffff:8.8.8.8') 'peer ::ffff:8.8.8.8'
}
Test-Case 'times, versions, hashes and MAC addresses are not mistaken for addresses' {
  foreach ($t in 'at 10:00:00 IST', 'date 2026-11-23 05:40:15 +05:30', 'version 10.0.26200', 'sha256 FF97D7539D5C354B287F2B94F333F931B655E7AB80292C73CDFEF52C3C51F5E4', 'mac aa:bb:cc:dd:ee:ff', 'build 1.2.3.4.5') {
    Assert-Equal (ConvertTo-WbPrivateMasked -Text $t) $t
  }
}
Test-Case 'several addresses in one line, mixed, are each judged on their own' {
  Assert-Equal (ConvertTo-WbPrivateMasked -Text 'a 192.168.1.9 b 8.8.4.4 c [fe80::2]:5 d 10.1.2.3.') "a $M b 8.8.4.4 c [$M]:5 d $M."
}
Test-Case 'masking works through the pipeline and keeps empty lines' {
  $out = @('x 10.0.0.5', '', 'y 1.1.1.1') | ConvertTo-WbPrivateMasked
  Assert-Equal $out.Count 3
  Assert-Equal $out[0] "x $M"
  Assert-Equal $out[1] ''
  Assert-Equal $out[2] 'y 1.1.1.1'
}
Test-Case 'Format-WbEndpoint brackets IPv6 so the port cannot join the address' {
  Assert-Equal (Format-WbEndpoint -Address 'fe80::1%12' -Port 443) '[fe80::1%12]:443'
  Assert-Equal (Format-WbEndpoint -Address '10.0.0.5' -Port 443) '10.0.0.5:443'
}
Test-Case 'the engine subcommand is read from the command line' {
  Assert-Equal (Get-WbEngineSubcommand -CommandLine '"C:\app\wb-engine-vulkan-x86_64-pc-windows-msvc.exe" info') 'info'
  Assert-Equal (Get-WbEngineSubcommand -CommandLine 'C:\app\wb-engine-avx2.exe batch --request C:\x\req.json') 'batch'
  Assert-Equal (Get-WbEngineSubcommand -CommandLine '"C:\app\wb-engine-avx2.exe" bench --model m') 'bench'
  Assert-Equal (Get-WbEngineSubcommand -CommandLine 'C:\app\other.exe batch') $null
  Assert-Equal (Get-WbEngineSubcommand -CommandLine '') $null
  Assert-Equal (Get-WbEngineSubcommand -CommandLine $null) $null
}

function Verdict([int]$Seen = 0, [int]$Gate = 0, [bool]$Alive = $true, [bool]$Cap = $true, [bool]$Db = $true, [bool]$Eng = $true) {
  Get-WbOfflineVerdict -NonLoopbackSeen $Seen -GateStrings $Gate -AppAlive $Alive -CapabilityJson $Cap -TranscriptDb $Db -EngineBeyondProbes $Eng
}
Test-Case 'all three proofs present and nothing seen -> PASS, exit 0' {
  $v = Verdict
  Assert-Equal $v.ExitCode 0
  if ($v.Verdict -notmatch '^verdict: PASS') { throw $v.Verdict }
}
Test-Case 'no capability.json -> INCONCLUSIVE naming it, non-zero exit' {
  $v = Verdict -Cap $false
  Assert-Equal $v.ExitCode 4
  if ($v.Verdict -notmatch '^verdict: INCONCLUSIVE \(no post-processing: capability\.json missing') { throw $v.Verdict }
  if ($v.Verdict -match 'transcript|engine process') { throw "named more than the missing proof: $($v.Verdict)" }
}
Test-Case 'no transcript.db -> INCONCLUSIVE naming it, non-zero exit' {
  $v = Verdict -Db $false
  Assert-Equal $v.ExitCode 4
  if ($v.Verdict -notmatch '^verdict: INCONCLUSIVE \(no post-processing: no transcript\.db') { throw $v.Verdict }
  if ($v.Verdict -match 'capability|engine process') { throw "named more than the missing proof: $($v.Verdict)" }
}
Test-Case 'no engine beyond the start-up probes -> INCONCLUSIVE naming it, non-zero exit' {
  $v = Verdict -Eng $false
  Assert-Equal $v.ExitCode 4
  if ($v.Verdict -notmatch '^verdict: INCONCLUSIVE \(no post-processing: no engine process beyond the start-up probes') { throw $v.Verdict }
  if ($v.Verdict -match 'capability|transcript') { throw "named more than the missing proof: $($v.Verdict)" }
}
Test-Case 'all three missing (the 22 Nov attempt) -> INCONCLUSIVE naming all three' {
  $v = Verdict -Cap $false -Db $false -Eng $false
  Assert-Equal $v.ExitCode 4
  foreach ($w in 'capability.json', 'transcript.db', 'engine process') { if ($v.Verdict -notmatch [regex]::Escape($w)) { throw "missing '$w' in $($v.Verdict)" } }
}
Test-Case 'a non-loopback socket offline stays FAIL even with every proof, and even with none' {
  $v = Verdict -Seen 1
  Assert-Equal $v.ExitCode 1
  Assert-Equal $v.Verdict 'verdict: FAIL'
  $v = Verdict -Seen 2 -Cap $false -Db $false -Eng $false
  Assert-Equal $v.ExitCode 1
  Assert-Equal $v.Verdict 'verdict: FAIL'
}
Test-Case 'a gate string in the exe, or the app exiting early, stays FAIL' {
  Assert-Equal (Verdict -Gate 3).ExitCode 1
  Assert-Equal (Verdict -Alive $false).ExitCode 1
}
Test-Case 'the masking never changes a verdict: classification uses the raw addresses' {
  # The check counts a private address as a non-loopback socket (a LAN peer is still a network), and only the
  # printed text is masked.
  $line = 'TCP 192.168.1.20:443 Established'
  Assert-Equal (ConvertTo-WbPrivateMasked -Text $line) "TCP $($M):443 Established"
  Assert-Equal (Verdict -Seen 1).ExitCode 1
}
if ($script:failures) { exit 1 }
