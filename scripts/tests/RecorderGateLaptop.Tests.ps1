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

# What runs beside the recorder follows the app: the quick check's model and CPU threads, else physical - 1 and logical / 2.
Import-Module (Join-Path $here '..\lib\RecorderGates.psm1') -Force
$bp = Join-Path ([IO.Path]::GetTempPath()) ('wb-beside-' + [guid]::NewGuid().ToString('N'))
try {
  New-Item -ItemType Directory -Force -Path (Join-Path $bp 'gates\quick-check'), (Join-Path $bp 'models') | Out-Null
  '{"physical_cores":8,"logical_processors":16}' | Set-Content -LiteralPath (Join-Path $bp 'fingerprint.json')
  $bctx = [pscustomobject]@{ pack_root = $bp; work_dir = $bp; profile = 'laptop' }
  $r0 = Get-PackBesideProduction -Ctx $bctx
  Check 'beside the recorder, without a quick check: batch physical minus 1, live half the logical, model medium' ($r0.batch_threads -eq 7 -and $r0.live_threads -eq 8 -and $r0.model -eq 'medium')
  New-Item -ItemType File -Force -Path (Join-Path $bp 'models\ggml-large-v3-turbo-q5_0.bin') | Out-Null
  '{"decision":{"config":{"config":{"kind":"gpu","index":0},"cpu_threads":2},"languages":{"en":{"rolling":{"model":"large-v3-turbo-q5_0"}}}}}' | Set-Content -LiteralPath (Join-Path $bp 'gates\quick-check\capability.json')
  $r1 = Get-PackBesideProduction -Ctx $bctx
  Check 'beside the recorder, with a quick check: its rolling model and its graphics-config CPU threads' ($r1.model -eq 'large-v3-turbo-q5_0' -and $r1.batch_threads -eq 2 -and $r1.live_threads -eq 8)
} finally { Remove-Item -LiteralPath $bp -Recurse -Force -ErrorAction SilentlyContinue }

if ($fails) { Write-Host "$fails failed"; exit 1 }
Write-Host 'RecorderGateLaptop tests passed'
