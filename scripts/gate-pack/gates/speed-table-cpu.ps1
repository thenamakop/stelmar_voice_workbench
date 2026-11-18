# Gate speed-table-cpu: wb-engine bench on the processor (avx2 at the physical-core and logical-processor
# thread counts; avx512 only where this CPU has AVX-512; small near-live), five models, English and
# Hindi, plus the code-switched clip for the Hinglish model. Information only.
# Windows PowerShell 5.1, ASCII only.
param([string]$CtxFile, [string]$ResultFile)
$ErrorActionPreference = 'Stop'
$lib = Join-Path $PSScriptRoot '..\..\lib'
Import-Module (Join-Path $lib 'PackCore.psm1') -Force
Import-Module (Join-Path $lib 'PackGate.psm1') -Force
$ctx = Read-PackCtx -CtxFile $CtxFile
Initialize-PackGate -Ctx $ctx
Import-Module (Join-Path $lib 'SpeedTable.psm1') -Force
Invoke-PackGateBody -Ctx $ctx -Id 'speed-table-cpu' -ResultFile $ResultFile -Body { Invoke-SpeedTableGate -Ctx $ctx -Part 'cpu' }
