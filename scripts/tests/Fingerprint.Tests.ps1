# Tests for scripts/lib/Fingerprint.psm1: integrated-graphics naming, same-disk rule, and a live read
# of this machine (the fields exist and nothing personal is in them).
# Run: powershell -NoProfile -ExecutionPolicy Bypass -File scripts/tests/Fingerprint.Tests.ps1
# ASCII only; Windows PowerShell 5.1.
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
Import-Module (Join-Path $here '..\lib\PackCore.psm1') -Force
Import-Module (Join-Path $here '..\lib\Fingerprint.psm1') -Force
$fails = 0
function Check([string]$Name, [bool]$Ok) {
  if ($Ok) { Write-Host "ok   $Name" } else { Write-Host "FAIL $Name"; $script:fails++ }
}

Check 'Intel UHD is integrated' (Test-PackIntegratedGpu -Name 'Intel(R) UHD Graphics 620')
Check 'Intel Iris Xe is integrated' (Test-PackIntegratedGpu -Name 'Intel(R) Iris(R) Xe Graphics')
Check 'AMD Radeon Graphics (an APU) is integrated' (Test-PackIntegratedGpu -Name 'AMD Radeon(TM) Graphics')
Check 'a GeForce RTX is not integrated' (-not (Test-PackIntegratedGpu -Name 'NVIDIA GeForce RTX 4070 Laptop GPU' -DedicatedBytes 4GB))
Check 'an Intel Arc A-series card is not integrated' (-not (Test-PackIntegratedGpu -Name 'Intel(R) Arc(TM) A370M Graphics'))
Check 'the Microsoft Basic Display Adapter is not a usable graphics adapter' (-not (Test-PackIntegratedGpu -Name 'Microsoft Basic Display Adapter'))
Check 'an unknown adapter with no dedicated memory is treated as integrated' (Test-PackIntegratedGpu -Name 'Mystery Graphics' -DedicatedBytes 0)

$c = [pscustomobject]@{ Letter = 'C'; Disk = 0 }; $d = [pscustomobject]@{ Letter = 'D'; Disk = 0 }; $e = [pscustomobject]@{ Letter = 'E'; Disk = 2 }
Check 'two drive letters on one physical disk are the same disk' (Test-PackSameDisk $c $d)
Check 'a second physical disk is a different disk' (-not (Test-PackSameDisk $c $e))
Check 'without disk numbers the drive letter decides' ((Test-PackSameDisk ([pscustomobject]@{ Letter = 'C'; Disk = $null }) ([pscustomobject]@{ Letter = 'C'; Disk = $null })) -and -not (Test-PackSameDisk ([pscustomobject]@{ Letter = 'C'; Disk = $null }) ([pscustomobject]@{ Letter = 'E'; Disk = $null })))

$fp = Get-PackFingerprint -WorkRoot $env:TEMP -BackupRoot $env:TEMP
Check 'the live fingerprint has a CPU, cores and RAM' ($fp.cpu -and $fp.logical_processors -gt 0 -and $fp.ram_gb -gt 1)
Check 'the live fingerprint says AVX2 (every machine the app supports has it)' ($fp.avx2 -eq $true)
Check 'the live fingerprint lists at least one graphics adapter with a driver' (@($fp.gpus).Count -ge 1 -and @($fp.gpus)[0].driver)
Check 'the machine name has no computer name or user name in it' ($fp.machine -notmatch [regex]::Escape($env:COMPUTERNAME) -and $fp.machine -notmatch [regex]::Escape($env:USERNAME))
Check 'two folders on one disk are reported as the same disk' ($fp.mirror_on_same_disk -eq $true)
Check 'the fingerprint lines mention the CPU and the graphics driver' ((Get-PackFingerprintLines -Fp $fp) -join "`n" -match 'CPU:' -and (Get-PackFingerprintLines -Fp $fp) -join "`n" -match 'driver')
Write-Host ('info this machine: {0}; AVX-512 {1}; {2} adapter(s); effects software: {3}' -f $fp.machine, $fp.avx512f, @($fp.gpus).Count, (@($fp.audio_effects_software) -join ', '))

if ($fails) { Write-Host "$fails failed"; exit 1 }
Write-Host 'Fingerprint tests passed'
