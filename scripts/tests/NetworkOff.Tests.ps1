# Tests for scripts/gates/network-off.ps1: it must refuse (exit code 2, before any engine runs) while any adapter is up.
# Run: powershell -NoProfile -ExecutionPolicy Bypass -File scripts/tests/NetworkOff.Tests.ps1
# No Pester needed. Exit code 1 when any case fails. ASCII only; Windows PowerShell 5.1.
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
Import-Module (Join-Path $here '..\lib\PsEnv.psm1') -Force
$script = Join-Path $here '..\gates\network-off.ps1'
$tmp = Join-Path $env:TEMP "wb-netoff-test-$PID"
New-Item -ItemType Directory -Force -Path $tmp | Out-Null
$script:failures = 0
function Test-Case([string]$Name, [scriptblock]$Body) {
  try { & $Body; Write-Host "PASS  $Name" } catch { $script:failures++; Write-Host "FAIL  $Name : $($_.Exception.Message)" }
}
function Invoke-Refusal([string]$Json) {
  $f = Join-Path $tmp 'facts.json'
  Set-Content -Path $f -Value $Json -Encoding ascii
  $out = @(Invoke-WbWindowsPowerShell -ArgumentList @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $script, '-FactsFile', $f, '-Out', (Join-Path $tmp 'out.txt')))
  [pscustomobject]@{ Exit = $LASTEXITCODE; Text = ($out -join "`n"); Wrote = (Test-Path (Join-Path $tmp 'out.txt')) }
}
$wifiUp = '{"Routes":[],"Interfaces":[],"Profiles":[],"PhysicalUp":[{"Name":"Wi-Fi","Description":"Wi-Fi adapter"}],"Errors":[]}'
$profileOnly = '{"Routes":[],"Interfaces":[],"Profiles":[{"ifIndex":9,"InterfaceAlias":"Ethernet","IPv4Connectivity":"Internet","IPv6Connectivity":"NoTraffic"}],"PhysicalUp":[],"Errors":[]}'
$localNet = '{"Routes":[],"Interfaces":[],"Profiles":[{"ifIndex":9,"InterfaceAlias":"Ethernet","IPv4Connectivity":"LocalNetwork","IPv6Connectivity":"NoTraffic"}],"PhysicalUp":[],"Errors":[]}'

Test-Case 'a physical adapter that is Up -> refused, exit 2, no evidence file, no engine run' {
  $r = Invoke-Refusal $wifiUp
  if ($r.Exit -ne 2) { throw "exit $($r.Exit), expected 2: $($r.Text)" }
  if ($r.Text -notmatch 'REFUSED') { throw 'no REFUSED line' }
  if ($r.Wrote) { throw 'an evidence file was written' }
  if ($r.Text -match '== avx2') { throw 'an engine section started' }
}
Test-Case 'a connection profile that reports Internet (adapter list empty) -> refused' {
  $r = Invoke-Refusal $profileOnly
  if ($r.Exit -ne 2 -or $r.Text -notmatch 'REFUSED') { throw "exit $($r.Exit): $($r.Text)" }
}
Test-Case 'a profile that reports LocalNetwork (a LAN counts as a network) -> refused' {
  $r = Invoke-Refusal $localNet
  if ($r.Exit -ne 2 -or $r.Text -notmatch 'REFUSED') { throw "exit $($r.Exit): $($r.Text)" }
}
Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
if ($script:failures) { exit 1 }
