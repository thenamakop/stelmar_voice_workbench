<#
  Fingerprint.psm1: what kind of computer this is, in the words the GATES Machines table needs, and a
  few facts the gates decide on (AVX-512, the integrated graphics adapter, which disk a folder is on).

  Everything is read from Windows; nothing is changed and nothing leaves the computer. The computer's
  own name and user name are never recorded (they can carry a person's name); the machine is named by
  maker, model and a short hash of the BIOS serial number (Get-PackMachineName in PackCore).

  Windows PowerShell 5.1, ASCII only.
#>

Import-Module (Join-Path $PSScriptRoot 'PackNative.psm1')

if (-not ('PackNative' -as [type])) {
  Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class PackNative {
  [DllImport("kernel32.dll")] public static extern bool IsProcessorFeaturePresent(uint feature);
}
'@
}

# PF_AVX2_INSTRUCTIONS_AVAILABLE = 40, PF_AVX512F_INSTRUCTIONS_AVAILABLE = 41 (Windows 10 2004 and later).
function Get-PackCpuFeatures {
  [pscustomobject]@{
    Avx2 = [PackNative]::IsProcessorFeaturePresent(40)
    Avx512f = [PackNative]::IsProcessorFeaturePresent(41)
    Sse42 = [PackNative]::IsProcessorFeaturePresent(38)
  }
}

# Pure. A video controller is integrated when its name says so or its maker is Intel (or AMD with an
# APU name) and it has no dedicated memory worth speaking of.
function Test-PackIntegratedGpu {
  param([string]$Name, [double]$DedicatedBytes = 0)
  if ($Name -match 'Microsoft Basic|Remote Display|Virtual|Parsec|Citrix') { return $false }
  if ($Name -match 'Arc(\(TM\))?\s+A\d') { return $false }
  if ($Name -match 'Intel.*(UHD|Iris|HD Graphics|Xe|Arc.*Graphics)') { return $true }
  if ($Name -match 'AMD Radeon.*Graphics$|Radeon\(TM\) Graphics|Radeon 6\d0M|Radeon 7\d0M|Vega \d+ Graphics') { return $true }
  if ($Name -match 'NVIDIA|GeForce|RTX|GTX|Radeon RX') { return $false }
  ($DedicatedBytes -lt 1GB)
}

function Get-PackGpus {
  $out = New-Object System.Collections.Generic.List[object]
  foreach ($v in @(Get-CimInstance Win32_VideoController -ErrorAction SilentlyContinue)) {
    $dedicated = $(if ($v.AdapterRAM) { [double]$v.AdapterRAM } else { 0.0 })
    $out.Add([pscustomobject]@{
        Name = [string]$v.Name; Driver = [string]$v.DriverVersion
        DriverDate = $(if ($v.DriverDate) { ([datetime]$v.DriverDate).ToString('yyyy-MM-dd') } else { '' })
        Integrated = (Test-PackIntegratedGpu -Name ([string]$v.Name) -DedicatedBytes $dedicated)
        Status = [string]$v.Status
      })
  }
  @($out.ToArray())
}

# Physical disk number and drive letter of a path (for "the mirror is on another disk").
function Get-PackDiskOf {
  param([Parameter(Mandatory = $true)] [string]$Path)
  $letter = ''
  try { $letter = (Split-Path -Qualifier $Path).TrimEnd(':').ToUpper() } catch { }
  $disk = $null; $bus = ''; $media = ''
  if ($letter) {
    try {
      $part = Get-Partition -DriveLetter $letter -ErrorAction Stop
      $disk = $part.DiskNumber
      $pd = Get-PhysicalDisk -ErrorAction SilentlyContinue | Where-Object { [string]$_.DeviceId -eq [string]$disk } | Select-Object -First 1
      if ($pd) { $bus = [string]$pd.BusType; $media = [string]$pd.MediaType }
    } catch { }
  }
  [pscustomobject]@{ Letter = $letter; Disk = $disk; BusType = $bus; MediaType = $media }
}

# Pure. Whether two folders are on the same physical disk (or, when the disk number is unknown, the
# same drive letter).
function Test-PackSameDisk {
  param($A, $B)
  if ($null -eq $A -or $null -eq $B) { return $true }
  if ($null -ne $A.Disk -and $null -ne $B.Disk) { return ($A.Disk -eq $B.Disk) }
  ($A.Letter -eq $B.Letter)
}

function Get-PackAudioEffectsSoftware {
  $found = New-Object System.Collections.Generic.List[string]
  $procRx = '^(RtkAudUService.*|RtkNGUI.*|RAVCpl.*|RAVBg.*|Nahimic.*|A-Volute.*|WavesSvc.*|WavesSysSvc.*|DolbyDAX.*|DAX.*|SmartAudio.*|IntelAudioService.*|SSTAudioService.*|MaxxAudio.*|DTS.*|SonicStudio.*|ASUSAudio.*)$'
  foreach ($p in @(Get-Process -ErrorAction SilentlyContinue | Where-Object { $_.ProcessName -match $procRx } | Select-Object -ExpandProperty ProcessName -Unique)) { $found.Add("process $p") }
  foreach ($s in @(Get-Service -ErrorAction SilentlyContinue | Where-Object { $_.Status -eq 'Running' -and ($_.Name -match 'Realtek|Nahimic|Waves|Dolby|IntelAudio|SSTAudio|Maxx|DTS|DAX' -or $_.DisplayName -match 'Realtek Audio|Nahimic|Waves|Dolby|Intel.*Smart Sound|Intel.*Audio') })) { $found.Add("service $($s.Name)") }
  @($found.ToArray() | Sort-Object -Unique)
}

# Whole fingerprint as a hashtable (JSON-friendly) and the lines for evidence.
function Get-PackFingerprint {
  [CmdletBinding()]
  param([string]$WorkRoot = '', [string]$BackupRoot = '')
  $cpu = Get-CimInstance Win32_Processor | Select-Object -First 1
  $cs = Get-CimInstance Win32_ComputerSystem
  $os = Get-CimInstance Win32_OperatingSystem
  $feat = Get-PackCpuFeatures
  $gpus = @(Get-PackGpus)
  $batt = @(Get-CimInstance Win32_Battery -ErrorAction SilentlyContinue)
  $plan = ''
  try { $plan = ((Invoke-PackNative -FilePath 'powercfg.exe' -ArgumentList @('/getactivescheme') -TimeoutSeconds 20).Output -join ' ') -replace '^.*:\s*[0-9a-f-]+\s*', '' } catch { }
  $fast = $null
  try { $fast = [int](Get-ItemPropertyValue 'HKLM:\SYSTEM\CurrentControlSet\Control\Session Manager\Power' -Name HiberbootEnabled -ErrorAction Stop) } catch { }
  $endpoints = @()
  try {
    $endpoints = @(Get-PnpDevice -Class AudioEndpoint -PresentOnly -ErrorAction Stop | ForEach-Object { [string]$_.FriendlyName } | Sort-Object -Unique)
  } catch { }
  $workDisk = $(if ($WorkRoot) { Get-PackDiskOf -Path $WorkRoot } else { $null })
  $backupDisk = $(if ($BackupRoot) { Get-PackDiskOf -Path $BackupRoot } else { $null })
  [ordered]@{
    machine = (Get-PackMachineName)
    maker = ([string]$cs.Manufacturer).Trim()
    model = ([string]$cs.Model).Trim()
    cpu = ([string]$cpu.Name).Trim()
    physical_cores = [int]$cpu.NumberOfCores
    logical_processors = [int]$cpu.NumberOfLogicalProcessors
    max_clock_mhz = [int]$cpu.MaxClockSpeed
    avx2 = [bool]$feat.Avx2
    avx512f = [bool]$feat.Avx512f
    ram_gb = [math]::Round([double]$cs.TotalPhysicalMemory / 1GB, 1)
    windows = ('{0} build {1}' -f ([string]$os.Caption).Trim(), [string]$os.BuildNumber)
    gpus = @($gpus | ForEach-Object { [ordered]@{ name = $_.Name; driver = $_.Driver; driver_date = $_.DriverDate; integrated = $_.Integrated } })
    has_battery = ($batt.Count -gt 0)
    power_plan = $plan
    fast_startup = $fast
    audio_endpoints = @($endpoints)
    audio_effects_software = @(Get-PackAudioEffectsSoftware)
    work_disk = $workDisk
    backup_disk = $backupDisk
    mirror_on_same_disk = $(if ($workDisk -and $backupDisk) { Test-PackSameDisk $workDisk $backupDisk } else { $null })
    taken = (Get-Date).ToString('o')
  }
}

function Get-PackFingerprintLines {
  param($Fp)
  $l = @()
  $l += ('  machine: {0} ({1} {2})' -f $Fp.machine, $Fp.maker, $Fp.model)
  $l += ('  CPU: {0}; {1} cores, {2} logical processors; up to {3} MHz; AVX2 {4}; AVX-512 {5}' -f $Fp.cpu, $Fp.physical_cores, $Fp.logical_processors, $Fp.max_clock_mhz, $(if ($Fp.avx2) { 'yes' } else { 'NO' }), $(if ($Fp.avx512f) { 'yes' } else { 'no' }))
  $l += ('  RAM: {0} GB; {1}; battery: {2}; power plan: {3}; Fast Startup: {4}' -f $Fp.ram_gb, $Fp.windows, $(if ($Fp.has_battery) { 'yes' } else { 'no' }), $Fp.power_plan, $(if ($null -eq $Fp.fast_startup) { 'unknown' } else { $Fp.fast_startup }))
  foreach ($g in @($Fp.gpus)) { $l += ('  graphics: {0}; driver {1} ({2}); {3}' -f $g.name, $g.driver, $g.driver_date, $(if ($g.integrated) { 'integrated' } else { 'discrete' })) }
  $l += ('  audio effects software running: {0}' -f $(if (@($Fp.audio_effects_software).Count -gt 0) { @($Fp.audio_effects_software) -join ', ' } else { 'none seen' }))
  if ($null -ne $Fp.mirror_on_same_disk) { $l += ('  recordings and mirror on the same physical disk: {0}' -f $(if ($Fp.mirror_on_same_disk) { 'YES (the mirror is not an independent copy)' } else { 'no (separate disks)' })) }
  $l
}

Export-ModuleMember -Function Get-PackCpuFeatures, Test-PackIntegratedGpu, Get-PackGpus, Get-PackDiskOf, Test-PackSameDisk, Get-PackAudioEffectsSoftware, Get-PackFingerprint, Get-PackFingerprintLines
