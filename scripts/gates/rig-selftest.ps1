<#
.SYNOPSIS
  Checks the virtual cable rig without running a gate.

.DESCRIPTION
  1. Lists every VB-CABLE endpoint Windows knows (active and stale) with the cable instance behind it.
  2. Refuses when more than one cable instance has active endpoints, and pairs the player's playback
     endpoint with the capture endpoint by cable instance (never by name).
  3. Refuses when the cable is a default playback device.
  4. Unless -ListOnly: plays a 6 s known signal through the pair and requires it on the capture.

  Every recorder gate and the gate sequence run steps 2 to 4 themselves before they start; this script
  is the same check on its own (about 15 s with the signal, nothing with -ListOnly). It records no
  evidence and takes no gate lock. Exit 0 = the rig is good; 1 = refused or failed.

    powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\gates\rig-selftest.ps1
#>
[CmdletBinding()]
param(
  [string]$Testbench,
  [string]$WorkRoot = 'D:\STELMAR-Gate',
  [string]$PlayDevice = '',
  [string]$CaptureDevice = '',
  [switch]$AllowCableDefault,
  [switch]$ListOnly
)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo = (Resolve-Path (Join-Path $here '..\..')).Path
if (-not $Testbench) { $Testbench = Join-Path $repo 'target\release\wb-testbench.exe' }
Import-Module (Join-Path $repo 'scripts\lib\PsEnv.psm1') -Force
Set-WbCleanPSModulePath
Set-WbInvariantCulture
Import-Module (Join-Path $repo 'scripts\lib\GateRun.psm1') -Force
Import-Module (Join-Path $repo 'scripts\lib\CableRig.psm1') -Force
if (-not (Test-Path -LiteralPath $Testbench)) { Write-Host "FAIL: missing $Testbench; build it first (scripts\gates\build-gate.ps1)"; exit 1 }

try {
  Write-Host 'VB-CABLE endpoints known to Windows (the instance is the device behind the endpoint):'
  $endpoints = @(Get-WbCableEndpoints)
  foreach ($e in $endpoints) { Write-Host ('  ' + (Format-WbCableEndpointLine $e)) }
  Write-Host 'VB-CABLE driver devices:'
  foreach ($d in @(Get-WbCableDriverDevices)) { Write-Host ('  {0} [{1}]' -f $d.InstanceId, $d.Status) }

  # No audio-enhancement program (Nahimic and the like) may be running in the audio path.
  Assert-WbNoResidentAudio
  Write-Host 'audio-enhancement programs (Nahimic, A-Volute, Waves, Dolby, Sonic Studio and the like): none running'
  $rig = Resolve-WbCableRig -Testbench $Testbench -PlayDevice $PlayDevice -CaptureDevice $CaptureDevice -Endpoints $endpoints
  Write-Host ''
  Write-Host "cable instance: $($rig.Instance)"
  Write-Host "capture endpoint (app and reference capture): $($rig.CaptureName)"
  Write-Host "playback endpoints of the same instance, in the order they are tried: $($rig.Candidates -join '; ')"
  foreach ($n in @($rig.Notes)) { Write-Host "note: $n" }
  $roles = @(Assert-WbCableNotDefault -Testbench $Testbench -Rig $rig -Allow:$AllowCableDefault)
  Write-Host $(if ($roles.Count) { "default playback device: THE CABLE for $($roles -join ', ') (allowed by -AllowCableDefault)" } else { 'default playback device: not the cable' })

  # Every audio session on the cable's playback endpoints now, by program. Anything playing refuses.
  $sessions = @(Get-WbCableSessions -Testbench $Testbench -Names $rig.RenderNames)
  if ($sessions.Count -eq 0) { Write-Host 'audio sessions on the cable now: none' }
  foreach ($s in $sessions) { Write-Host ('audio session on the cable: {0}, {1}' -f (Format-WbCableSession $s), $(if ($s.Active) { 'PLAYING' } else { 'silent' })) }
  $playing = @($sessions | Where-Object { $_.Active })
  if ($playing.Count -gt 0) { throw ('RIG REFUSED: another program is playing into the cable: ' + ((@($playing) | ForEach-Object { Format-WbCableSession $_ }) -join '; ') + '. Stop it so only the gate player feeds the cable.') }

  if ($ListOnly) { Write-Host 'RIG OK (listing only; no signal was played)'; exit 0 }
  $t = Invoke-WbRigSelfTest -Rig $rig -Testbench $Testbench -WorkRoot $WorkRoot
  foreach ($l in (Get-WbCableRigLines -Rig $rig -SelfTest $t)) { Write-Host $l.Trim() }
  Write-Host "RIG OK: play into '$($t.PlayName)', record '$($t.CaptureName)', cable instance $($t.Instance)"
  exit 0
} catch {
  Write-Host "$($_.Exception.Message)"
  exit 1
}
