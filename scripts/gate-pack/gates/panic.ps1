# Gate panic: the panic recorder gate on the hardware-interface loopback (VB-CABLE only as a loudly
# marked fallback), with the app judged against two independent references (cpal and raw WASAPI), the
# detector rates, and the thermal summary where it applies. Wraps scripts\gates\recorder-gate.ps1.
# Windows PowerShell 5.1, ASCII only.
param([string]$CtxFile, [string]$ResultFile)
$ErrorActionPreference = 'Stop'
$lib = Join-Path $PSScriptRoot '..\..\lib'
Import-Module (Join-Path $lib 'PackCore.psm1') -Force
Import-Module (Join-Path $lib 'PackGate.psm1') -Force
$ctx = Read-PackCtx -CtxFile $CtxFile
Initialize-PackGate -Ctx $ctx
Import-Module (Join-Path $lib 'RecorderGates.psm1') -Force
Invoke-PackGateBody -Ctx $ctx -Id 'panic' -ResultFile $ResultFile -Body {
  Invoke-PackRecorderGate -Ctx $ctx -Id 'panic' -Gate 'panic' -FullMinutes 3 -QuickMinutes 3  -Threshold 'zero samples lost; recorder restarts once; equals both references'
}
