# Gate bg-decode-info: information only. Two recordings on the audio interface, the same length, one with
# nothing beside the recorder and one with a batch engine decoding in the background; the shared upstream
# breaks (the source chain broke, the reference capture shows them too) are counted in each. A break only
# the app shows is a recorder finding and fails the run. Windows PowerShell 5.1, ASCII only.
param([string]$CtxFile, [string]$ResultFile)
$ErrorActionPreference = 'Stop'
$lib = Join-Path $PSScriptRoot '..\..\lib'
Import-Module (Join-Path $lib 'PackCore.psm1') -Force
Import-Module (Join-Path $lib 'PackGate.psm1') -Force
$ctx = Read-PackCtx -CtxFile $CtxFile
Initialize-PackGate -Ctx $ctx
Import-Module (Join-Path $lib 'RecorderGates.psm1') -Force

Invoke-PackGateBody -Ctx $ctx -Id 'bg-decode-info' -ResultFile $ResultFile -Body {
  $quick = [bool]$ctx.quick
  $minutes = $(if ($quick) { 5 } else { 30 })
  $dev = Get-PackBesideDevice -Ctx $ctx
  $thr = 'information only: shared upstream breaks with and without a background decode; a break the app shows alone is a recorder finding'
  $a = Invoke-PackRecorderGate -Ctx $ctx -Id 'bg-decode-info' -Gate 'soak' -FullMinutes 30 -QuickMinutes 5 -RunTag 'without-decode' -Threshold $thr
  if ($a['status'] -eq 'PENDING' -or $null -eq $a['measured']) { return $a }
  $b = Invoke-PackRecorderGate -Ctx $ctx -Id 'bg-decode-info' -Gate 'soak' -FullMinutes 30 -QuickMinutes 5 -RunTag 'with-decode' -Beside @{ mode = 'batch'; batch_variant = $dev.batch_variant; batch_device = $dev.batch_device; batch_threads = $dev.batch_threads; model = $dev.model } -Threshold $thr
  if ($b['status'] -eq 'PENDING' -or $null -eq $b['measured']) { return $b }
  $sa = [int]$a['measured']['shared_breaks']; $sb = [int]$b['measured']['shared_breaks']
  $oa = [int]$a['measured']['app_only_breaks']; $ob = [int]$b['measured']['app_only_breaks']
  $status = 'INFO'
  if ($a['status'] -eq 'FAIL' -or $b['status'] -eq 'FAIL') { $status = 'FAIL' }
  elseif ($a['status'] -ne 'PASS' -or $b['status'] -ne 'PASS') { $status = 'INVALID' }
  $head = ('shared upstream breaks over {0} minutes: {1} without a background decode, {2} with one ({3}); breaks only in the app: {4} and {5}' -f $minutes, $sa, $sb, $dev.device_note, $oa, $ob)
  $sum = Join-Path ([string]$ctx.gate_dir) 'bg-decode-info-summary.txt'
  Set-Content -LiteralPath $sum -Encoding UTF8 -Value @(
    'bg-decode-info: the same recording twice, without and with a batch engine decoding in the background',
    ('  minutes per run: {0}; batch engine: wb-engine {1} on {2}, model medium, below-normal priority' -f $minutes, $dev.batch_variant, $dev.device_note),
    ('  without a decode: {0} (rig {1}); shared upstream breaks {2}; app-only breaks {3}' -f $a['status'], $a['measured']['rig'], $sa, $oa),
    ('  with a decode:    {0} (rig {1}); shared upstream breaks {2}; app-only breaks {3}' -f $b['status'], $b['measured']['rig'], $sb, $ob),
    '  information only: two runs are too few to say a decode raises or lowers the number; the owner reads the counts next to the earlier rig runs')
  $meas = [ordered]@{ minutes = $minutes; shared_without = $sa; shared_with = $sb; app_only_without = $oa; app_only_with = $ob; rig = $b['measured']['rig']; batch_device = $dev.device_note }
  New-PackResult -Id 'bg-decode-info' -Status $status -Headline $head -Measured $meas -Threshold $thr -Rig ([string]$b['rig']) -Evidence @(@($a['evidence']) + @($b['evidence']) + @($sum)) -Notes (@($a['notes']) + @($b['notes'])) -Quick $quick -SharedBreaks ($sa + $sb)
}
