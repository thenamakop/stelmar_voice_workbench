# Gate kill: force-kill the app 5 times mid-recording (2 with -Quick) on the hardware-interface loopback,
# with the app judged against two references and the capture margin of every run printed.
# Wraps scripts\gates\kill-gate.ps1. Windows PowerShell 5.1, ASCII only.
param([string]$CtxFile, [string]$ResultFile)
$ErrorActionPreference = 'Stop'
$lib = Join-Path $PSScriptRoot '..\..\lib'
Import-Module (Join-Path $lib 'PackCore.psm1') -Force
Import-Module (Join-Path $lib 'PackGate.psm1') -Force
$ctx = Read-PackCtx -CtxFile $CtxFile
Initialize-PackGate -Ctx $ctx
Import-Module (Join-Path $lib 'RecorderGates.psm1') -Force
Invoke-PackGateBody -Ctx $ctx -Id 'kill' -ResultFile $ResultFile -Body {
  Invoke-PackRecorderGate -Ctx $ctx -Id 'kill' -Gate 'kill' -FullKills 5 -QuickKills 2 -Threshold 'at most 1 s lost per kill; every part plays before repair; restart offers Resume; each run equals both references; zero overruns; capture margin of every run printed'
}
