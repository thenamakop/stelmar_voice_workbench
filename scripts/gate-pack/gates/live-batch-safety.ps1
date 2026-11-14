# Gate live-batch-safety: the recorder on the audio interface for 15 minutes with a live engine and a batch
# engine running beside it. The live engine (wb-engine live) is voice detection only until Phase 7 adds
# decoding, so this proves the recorder next to a real batch engine and a live engine's CPU use, and is
# repeated when Phase 7 lands. Wraps scripts\gates\recorder-gate.ps1 -Gate soak with a beside-load script.
# Windows PowerShell 5.1, ASCII only.
param([string]$CtxFile, [string]$ResultFile)
$ErrorActionPreference = 'Stop'
$lib = Join-Path $PSScriptRoot '..\..\lib'
Import-Module (Join-Path $lib 'PackCore.psm1') -Force
Import-Module (Join-Path $lib 'PackGate.psm1') -Force
$ctx = Read-PackCtx -CtxFile $CtxFile
Initialize-PackGate -Ctx $ctx
Import-Module (Join-Path $lib 'RecorderGates.psm1') -Force

Invoke-PackGateBody -Ctx $ctx -Id 'live-batch-safety' -ResultFile $ResultFile -Body {
  $dev = Get-PackBesideDevice -Ctx $ctx
  $cfg = @{ mode = 'live-batch'; batch_variant = $dev.batch_variant; batch_device = $dev.batch_device; batch_threads = $dev.batch_threads }
  $r = Invoke-PackRecorderGate -Ctx $ctx -Id 'live-batch-safety' -Gate 'soak' -FullMinutes 15 -QuickMinutes 5 -Beside $cfg -Threshold 'zero overruns, xruns and dropped samples; recording equals both references while wb-engine live (voice detection only until Phase 7) and a batch engine run beside the recorder'
  if ($null -eq $r['measured']) { return $r }
  $r['measured']['beside'] = [ordered]@{ live = 'wb-engine live, voice detection only, 2 threads, normal priority'; batch = ('wb-engine batch, model medium, {0}, below-normal priority' -f $dev.device_note) }
  $r['notes'] = @($r['notes']) + @('the live engine decodes nothing yet (Phase 7); run this gate again when decoding lands')
  $r['headline'] = ('{0} [beside the recorder: live engine (voice detection) and batch engine on {1}]' -f $r['headline'], $dev.device_note)
  $r
}
