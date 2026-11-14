# Gate speed-table-igpu: wb-engine bench on every real Vulkan graphics adapter of this laptop (driver
# version in every row), five models, English and Hindi, plus the code-switched clip for the Hinglish
# model. Information only (Phase 2 sets no speed bar). Windows PowerShell 5.1, ASCII only.
param([string]$CtxFile, [string]$ResultFile)
$ErrorActionPreference = 'Stop'
$lib = Join-Path $PSScriptRoot '..\..\lib'
Import-Module (Join-Path $lib 'PackCore.psm1') -Force
Import-Module (Join-Path $lib 'PackGate.psm1') -Force
$ctx = Read-PackCtx -CtxFile $CtxFile
Initialize-PackGate -Ctx $ctx
Import-Module (Join-Path $lib 'SpeedTable.psm1') -Force
Invoke-PackGateBody -Ctx $ctx -Id 'speed-table-igpu' -ResultFile $ResultFile -Body { Invoke-SpeedTableGate -Ctx $ctx -Part 'igpu' }
