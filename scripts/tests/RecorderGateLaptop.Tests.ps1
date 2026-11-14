# Tests for the laptop mode of scripts/gates/recorder-gate.ps1 (-Profile laptop, -RigPrefer, -SecondReference):
# the parameters exist, wb-emulate is not needed, and the old profiles still demand it.
# Run: powershell -NoProfile -ExecutionPolicy Bypass -File scripts/tests/RecorderGateLaptop.Tests.ps1
# ASCII only; Windows PowerShell 5.1.
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$gate = (Resolve-Path (Join-Path $here '..\gates\recorder-gate.ps1')).Path
Import-Module (Join-Path $here '..\lib\PackNative.psm1') -Force
$fails = 0
function Check([string]$Name, [bool]$Ok) {
  if ($Ok) { Write-Host "ok   $Name" } else { Write-Host "FAIL $Name"; $script:fails++ }
}
$ast = [System.Management.Automation.Language.Parser]::ParseFile($gate, [ref]$null, [ref]$null)
$params = @{}
foreach ($p in $ast.ParamBlock.Parameters) { $params[$p.Name.VariablePath.UserPath] = $p }
Check 'the laptop profile is accepted' ($params['Profile'].Attributes.PositionalArguments.Extent.Text -match "'laptop'" -or $params['Profile'].Extent.Text -match "'laptop'")
Check '-RigPrefer takes loopback or cable' ($params.ContainsKey('RigPrefer') -and $params['RigPrefer'].Extent.Text -match "'loopback'" -and $params['RigPrefer'].Extent.Text -match "'cable'")
Check '-SecondReference is a switch' ($params.ContainsKey('SecondReference') -and $params['SecondReference'].StaticType.Name -eq 'SwitchParameter')
Check 'the default profile is still emu-low' ($params['Profile'].DefaultValue.Value -eq 'emu-low')

$kill = (Resolve-Path (Join-Path $here '..\gates\kill-gate.ps1')).Path
$kast = [System.Management.Automation.Language.Parser]::ParseFile($kill, [ref]$null, [ref]$null)
$kp = @{}
foreach ($p in $kast.ParamBlock.Parameters) { $kp[$p.Name.VariablePath.UserPath] = $p }
Check 'kill-gate accepts the laptop profile, -RigPrefer and -SecondReference' ($kp['Profile'].Extent.Text -match "'laptop'" -and $kp.ContainsKey('RigPrefer') -and $kp.ContainsKey('SecondReference'))
Check 'kill-gate: the default profile is still emu-low' ($kp['Profile'].DefaultValue.Value -eq 'emu-low')

$tmp = Join-Path ([IO.Path]::GetTempPath()) ('wb-rg-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path $tmp | Out-Null
try {
  $fake = Join-Path $tmp 'fake.exe'
  Set-Content -LiteralPath $fake -Value 'x' -Encoding ascii
  $ps = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
  function Run([string[]]$a) {
    $r = Invoke-PackNative -FilePath $ps -ArgumentList (@('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $gate) + $a) -TimeoutSeconds 90
    ((@($r.Output) + @($r.Error)) -join "`n")
  }
  # Laptop: the app and wb-testbench are needed, wb-emulate is not.
  $t = Run @('-Gate', 'soak', '-Profile', 'laptop', '-Exe', (Join-Path $tmp 'no-app.exe'), '-Testbench', $fake, '-WorkRoot', $tmp, '-BackupRoot', $tmp, '-OutDir', $tmp)
  Check 'laptop: a missing app is reported' ($t -match 'Missing .*no-app\.exe')
  Check 'laptop: wb-emulate is never asked for' ($t -notmatch 'wb-emulate')
  $t = Run @('-Gate', 'soak', '-Profile', 'laptop', '-Exe', $fake, '-Testbench', (Join-Path $tmp 'no-tb.exe'), '-WorkRoot', $tmp, '-BackupRoot', $tmp, '-OutDir', $tmp)
  Check 'laptop: a missing wb-testbench is reported' ($t -match 'Missing .*no-tb\.exe')
  # The emulated profiles still need the emulator.
  $t = Run @('-Gate', 'soak', '-Profile', 'emu-low', '-Exe', $fake, '-Testbench', $fake, '-Emulate', (Join-Path $tmp 'no-emu.exe'), '-WorkRoot', $tmp, '-BackupRoot', $tmp, '-OutDir', $tmp)
  Check 'emu-low: a missing wb-emulate is still reported' ($t -match 'Missing .*no-emu\.exe')
} finally { Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue }

if ($fails) { Write-Host "$fails failed"; exit 1 }
Write-Host 'RecorderGateLaptop tests passed'
