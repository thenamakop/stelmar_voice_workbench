# Tests for scripts/lib/CableRig.psm1: the cable rig of the recorder gates (no audio is played).
# Run: powershell -NoProfile -ExecutionPolicy Bypass -File scripts/tests/CableRig.Tests.ps1
# ASCII only; Windows PowerShell 5.1.
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
Import-Module (Join-Path $here '..\lib\PsEnv.psm1') -Force
Import-Module (Join-Path $here '..\lib\GateRun.psm1') -Force
Import-Module (Join-Path $here '..\lib\CableRig.psm1') -Force
$fails = 0
function Check([string]$Name, [bool]$Ok) {
  if ($Ok) { Write-Host "ok   $Name" } else { Write-Host "FAIL $Name"; $script:fails++ }
}
# True when the body throws "RIG REFUSED: ..." and the message matches every given text.
function Refuses([scriptblock]$Body, [string[]]$Texts = @()) {
  try { $null = & $Body; return $false } catch {
    $m = $_.Exception.Message
    if ($m -notmatch '^RIG REFUSED: ') { Write-Host "     (message was: $m)"; return $false }
    foreach ($t in $Texts) { if ($m.IndexOf($t, [StringComparison]::OrdinalIgnoreCase) -lt 0) { Write-Host "     (message lacks '$t': $m)"; return $false } }
    return $true
  }
}
$emptyContainer = '{00000000-0000-0000-FFFF-FFFFFFFFFFFF}'
function Ep([string]$Flow, [string]$Name, [string]$Instance, [int]$State = 1) {
  $g = '{' + [guid]::NewGuid().ToString() + '}'
  $prefix = $(if ($Flow -eq 'Render') { '{0.0.0.00000000}' } else { '{0.0.1.00000000}' })
  [pscustomobject]@{ Flow = $Flow; Guid = $g; EndpointId = ($prefix + '.' + $g); State = $State; Active = ($State -eq 1); FriendlyName = $Name; ShortName = ''; Instance = $Instance; ContainerId = $emptyContainer; KsInterface = '' }
}
# What Windows listed on 12 Nov 2026 after the restart: three driver instances, one working.
function TodaysEndpoints {
  @(
    (Ep 'Capture' 'CABLE Output (VB-Audio Virtual Cable)' 'ROOT\MEDIA\0001' 4)
    (Ep 'Render' 'CABLE Input (VB-Audio Virtual Cable)' 'ROOT\MEDIA\0001' 4)
    (Ep 'Render' 'CABLE In 16ch (VB-Audio Virtual Cable)' 'ROOT\MEDIA\0001' 4)
    (Ep 'Capture' 'CABLE Output (2- VB-Audio Virtual Cable)' 'ROOT\MEDIA\0003' 4)
    (Ep 'Render' 'Speakers (2- VB-Audio Virtual Cable)' 'ROOT\MEDIA\0003' 4)
    (Ep 'Render' 'CABLE In 16 Ch (2- VB-Audio Virtual Cable)' 'ROOT\MEDIA\0003' 4)
    (Ep 'Capture' 'CABLE Output (3- VB-Audio Virtual Cable)' 'ROOT\MEDIA\0002')
    (Ep 'Render' 'Speakers (3- VB-Audio Virtual Cable)' 'ROOT\MEDIA\0002')
    (Ep 'Render' 'CABLE In 16 Ch (3- VB-Audio Virtual Cable)' 'ROOT\MEDIA\0002')
  )
}
function DevicesFor($Endpoints, [string]$Default = 'Speakers (Realtek(R) Audio)') {
  $cap = @($Endpoints | Where-Object { $_.Active -and $_.Flow -eq 'Capture' })
  [pscustomobject]@{
    inputs = @($cap | ForEach-Object { [pscustomobject]@{ name = $_.FriendlyName; id = ('wasapi:' + $_.EndpointId) } }) + @([pscustomobject]@{ name = 'Microphone Array (Realtek(R) Audio)'; id = 'wasapi:{0.0.1.00000000}.{848816e6-87df-41b8-9da5-260ee6f2215e}' })
    outputs = @(@($Endpoints | Where-Object { $_.Active -and $_.Flow -eq 'Render' } | ForEach-Object { $_.FriendlyName }) + @('Speakers (Realtek(R) Audio)', 'Speakers (Steam Streaming Speakers)'))
    default_roles = [pscustomobject]@{ console = $Default; multimedia = $Default; communications = $Default }
  }
}
$driversOk = @([pscustomobject]@{ InstanceId = 'ROOT\MEDIA\0002'; Status = 'OK'; Name = 'VB-Audio Virtual Cable' })
$tb = 'unused-in-these-tests.exe'

# ---- The layout of 12 Nov 2026 ---------------------------------------------------------------------
$eps = TodaysEndpoints
$rig = Resolve-WbCableRig -Testbench $tb -Endpoints $eps -Devices (DevicesFor $eps) -DriverDevices $driversOk
Check 'today: the one working cable instance is found' ($rig.Instance -eq 'ROOT\MEDIA\0002')
Check 'today: the capture endpoint is the active one, with its "3-" name' ($rig.CaptureName -eq 'CABLE Output (3- VB-Audio Virtual Cable)')
Check 'today: the playback candidates are the active endpoints of that instance, the stereo one first' ((@($rig.Candidates) -join '|') -eq 'Speakers (3- VB-Audio Virtual Cable)|CABLE In 16 Ch (3- VB-Audio Virtual Cable)')
Check 'today: stale entries ("CABLE Input", the "2-" endpoints) are never candidates' (@($rig.Candidates | Where-Object { $_ -match 'CABLE Input|\(2-' }).Count -eq 0)
Check 'today: stale entries are mentioned as a note' (@($rig.Notes | Where-Object { $_ -match 'stale' }).Count -eq 1)
Check 'today: a cable driver device in an error state is mentioned' (@((Resolve-WbCableRig -Testbench $tb -Endpoints $eps -Devices (DevicesFor $eps) -DriverDevices @([pscustomobject]@{ InstanceId = 'ROOT\MEDIA\0001'; Status = 'Error'; Name = 'VB-Audio Virtual Cable' })).Notes | Where-Object { $_ -match 'ROOT\\MEDIA\\0001 \[Error\]' }).Count -eq 1)
Check 'today: asking for the old name "CABLE Input" is refused (it is a stale entry, not a device)' (Refuses { Resolve-WbCableRig -Testbench $tb -PlayDevice 'CABLE Input' -Endpoints $eps -Devices (DevicesFor $eps) -DriverDevices $driversOk } @('CABLE Input', 'not an active VB-CABLE Render endpoint', 'Speakers (3-'))
$named = Resolve-WbCableRig -Testbench $tb -PlayDevice 'CABLE In 16 Ch (3- VB-Audio Virtual Cable)' -CaptureDevice 'CABLE Output (3-' -Endpoints $eps -Devices (DevicesFor $eps) -DriverDevices $driversOk
Check 'a requested playback endpoint is used when it is on the capture endpoint''s cable' ((@($named.Candidates) -join '|') -eq 'CABLE In 16 Ch (3- VB-Audio Virtual Cable)' -and $named.CaptureName -eq 'CABLE Output (3- VB-Audio Virtual Cable)')

# ---- More than one cable instance: refuse and name them ---------------------------------------------
$two = @($eps) + @((Ep 'Capture' 'CABLE Output (4- VB-Audio Virtual Cable)' 'ROOT\MEDIA\0004'), (Ep 'Render' 'Speakers (4- VB-Audio Virtual Cable)' 'ROOT\MEDIA\0004'))
Check 'two cable instances with active endpoints: refused, naming both instances and their endpoints' (Refuses { Resolve-WbCableRig -Testbench $tb -Endpoints $two -Devices (DevicesFor $two) -DriverDevices $driversOk } @('ROOT\MEDIA\0002', 'ROOT\MEDIA\0004', 'CABLE Output (3-', 'CABLE Output (4-', 'Speakers (4-', 'more than one VB-CABLE instance'))
$twoNamed = @{ Testbench = $tb; PlayDevice = 'Speakers (4- VB-Audio Virtual Cable)'; CaptureDevice = 'CABLE Output (3- VB-Audio Virtual Cable)'; Endpoints = $two; Devices = (DevicesFor $two); DriverDevices = $driversOk }
Check 'two instances and a player and a capture named from different ones: refused' (Refuses { Resolve-WbCableRig @twoNamed } @('more than one VB-CABLE instance'))

# ---- A mismatched pair is refused -------------------------------------------------------------------
$playA = Ep 'Render' 'Speakers (3- VB-Audio Virtual Cable)' 'ROOT\MEDIA\0002'
$capA = Ep 'Capture' 'CABLE Output (3- VB-Audio Virtual Cable)' 'ROOT\MEDIA\0002'
$capB = Ep 'Capture' 'CABLE Output (4- VB-Audio Virtual Cable)' 'ROOT\MEDIA\0004'
$accepted = $false
try { Assert-WbCablePair -Play $playA -Capture $capA; $accepted = $true } catch { Write-Host "     ($($_.Exception.Message))" }
Check 'a pair on the same cable instance is accepted' $accepted
Check 'a mismatched pair (player on one cable instance, capture on another) is refused, naming both' (Refuses { Assert-WbCablePair -Play $playA -Capture $capB } @('mismatched pair', 'ROOT\MEDIA\0002', 'ROOT\MEDIA\0004', 'Speakers (3-', 'CABLE Output (4-'))
Check 'the same pair names with different "N-" prefixes are not enough: only the instance counts' (Refuses { Assert-WbCablePair -Play (Ep 'Render' 'CABLE Input (3- VB-Audio Virtual Cable)' 'ROOT\MEDIA\0001') -Capture $capA } @('mismatched pair'))
Check 'a pair whose instance is unknown is refused' (Refuses { Assert-WbCablePair -Play (Ep 'Render' 'X' '') -Capture $capA } @('cannot tell which cable instance'))
Check 'a capture endpoint given as the player is refused' (Refuses { Assert-WbCablePair -Play $capA -Capture $capA } @('not a playback'))
Check 'a playback endpoint given as the capture is refused' (Refuses { Assert-WbCablePair -Play $playA -Capture $playA } @('not a recording'))

# ---- Other refusals ---------------------------------------------------------------------------------
Check 'no active cable endpoint: refused' (Refuses { Resolve-WbCableRig -Testbench $tb -Endpoints @($eps | Where-Object { -not $_.Active }) -Devices (DevicesFor $eps) -DriverDevices $driversOk } @('no active VB-CABLE'))
Check 'an active endpoint with an unreadable cable instance: refused' (Refuses { Resolve-WbCableRig -Testbench $tb -Endpoints @((Ep 'Capture' 'CABLE Output (3- VB-Audio Virtual Cable)' ''), $playA) -Devices (DevicesFor $eps) -DriverDevices $driversOk } @('cannot read'))
Check 'a cable with no playback endpoint: refused' (Refuses { Resolve-WbCableRig -Testbench $tb -Endpoints @($capA) -Devices (DevicesFor @($capA)) -DriverDevices $driversOk } @('no active playback'))
Check 'a cable with no capture endpoint: refused' (Refuses { Resolve-WbCableRig -Testbench $tb -Endpoints @($playA) -Devices (DevicesFor @($playA)) -DriverDevices $driversOk } @('no active capture'))
$amb = DevicesFor $eps
$amb.outputs = @($amb.outputs) + @('Speakers (3- VB-Audio Virtual Cable) copy')
Check 'a playback name the player would match twice: refused' (Refuses { Resolve-WbCableRig -Testbench $tb -Endpoints $eps -Devices $amb -DriverDevices $driversOk } @('more than one playback device'))
$ambIn = DevicesFor $eps
$ambIn.inputs = @($ambIn.inputs) + @([pscustomobject]@{ name = 'CABLE Output (3- VB-Audio Virtual Cable) extra'; id = 'wasapi:{0.0.1.00000000}.{aaaaaaaa-0000-0000-0000-000000000000}' })
Check 'a capture name the app would match twice: refused' (Refuses { Resolve-WbCableRig -Testbench $tb -Endpoints $eps -Devices $ambIn -DriverDevices $driversOk } @('more than one recording device'))
$wrongId = DevicesFor $eps
$wrongId.inputs = @($wrongId.inputs | ForEach-Object { if ($_.name -like 'CABLE Output (3-*') { [pscustomobject]@{ name = $_.name; id = 'wasapi:{0.0.1.00000000}.{bbbbbbbb-0000-0000-0000-000000000000}' } } else { $_ } })
Check 'a capture endpoint whose id differs from the cable instance''s endpoint: refused' (Refuses { Resolve-WbCableRig -Testbench $tb -Endpoints $eps -Devices $wrongId -DriverDevices $driversOk } @('has id'))
$blind = DevicesFor $eps
$blind.outputs = @('Speakers (Realtek(R) Audio)')
Check 'no cable playback endpoint visible to the player: refused' (Refuses { Resolve-WbCableRig -Testbench $tb -Endpoints $eps -Devices $blind -DriverDevices $driversOk } @('none of the playback endpoints'))

# ---- The default playback device -------------------------------------------------------------------
$cableDefault = DevicesFor $eps 'Speakers (3- VB-Audio Virtual Cable)'
Check 'the old default list catches today''s cable default ("Speakers (3- VB-Audio Virtual Cable)" has no "CABLE Input" in it)' ((@(Get-WbCableDefaultRolesFromDevices -Devices $cableDefault) -join ',') -eq 'console,multimedia,communications')
Check 'a real default output is not the cable' (@(Get-WbCableDefaultRolesFromDevices -Devices (DevicesFor $eps)).Count -eq 0)
Check 'one role on the cable is reported alone' ((@(Get-WbCableDefaultRolesFromDevices -Devices ([pscustomobject]@{ default_roles = [pscustomobject]@{ console = 'Speakers (Realtek(R) Audio)'; multimedia = 'Speakers (Realtek(R) Audio)'; communications = 'CABLE In 16 Ch (3- VB-Audio Virtual Cable)' } })) -join ',') -eq 'communications')
Check 'the default device on the cable is refused, with the real outputs named' (Refuses { Assert-WbCableNotDefault -Testbench $tb -Rig $rig -Devices $cableDefault } @('default playback device', 'ROOT\MEDIA\0002', 'Speakers (Realtek(R) Audio)', 'communications'))
Check '-Allow lets it through and still reports the roles' ((@(Assert-WbCableNotDefault -Testbench $tb -Rig $rig -Devices $cableDefault -Allow) -join ',') -eq 'console,multimedia,communications')
Check 'a real default output passes' (@(Assert-WbCableNotDefault -Testbench $tb -Rig $rig -Devices (DevicesFor $eps)).Count -eq 0)

# ---- The signal check's verdict ---------------------------------------------------------------------
$good = [pscustomobject]@{ content_start_ms = 80.0; min_block_correlation = 1.0 }
$level = [pscustomobject]@{ Rms = 4729.0; Dbfs = -16.8 }
Check 'verdict: a signal found early, correlated and loud passes' ((Get-WbRigToneVerdict -Analysis $good -Level $level).Passed)
Check 'verdict: signal not found fails, with the analyser''s text' ((-not (Get-WbRigToneVerdict -Analysis $null -AnalysisError 'wb-testbench: the test signal was not found in the recording').Passed) -and (Get-WbRigToneVerdict -Analysis $null -AnalysisError 'wb-testbench: the test signal was not found in the recording').Reason -match 'not found')
Check 'verdict: a signal that starts after 10 s fails' (-not (Get-WbRigToneVerdict -Analysis ([pscustomobject]@{ content_start_ms = 10500.0; min_block_correlation = 1.0 }) -Level $level).Passed)
Check 'verdict: exactly 10 s still passes' ((Get-WbRigToneVerdict -Analysis ([pscustomobject]@{ content_start_ms = 10000.0; min_block_correlation = 1.0 }) -Level $level).Passed)
Check 'verdict: a weak correlation fails' (-not (Get-WbRigToneVerdict -Analysis ([pscustomobject]@{ content_start_ms = 80.0; min_block_correlation = 0.85 }) -Level $level).Passed)
Check 'verdict: a signal that arrives too quietly fails' (-not (Get-WbRigToneVerdict -Analysis $good -Level ([pscustomobject]@{ Rms = 300.0; Dbfs = -40.8 })).Passed)
Check 'verdict: an unreadable level fails' (-not (Get-WbRigToneVerdict -Analysis $good -Level $null).Passed)

# ---- Reading a level from a WAV --------------------------------------------------------------------
$wav = Join-Path ([IO.Path]::GetTempPath()) ('wb-rig-level-' + [guid]::NewGuid().ToString('N') + '.wav')
try {
  $frames = 48000
  $ms = New-Object System.IO.MemoryStream
  $bw = New-Object System.IO.BinaryWriter($ms)
  $bw.Write([Text.Encoding]::ASCII.GetBytes('RIFF')); $bw.Write([uint32](36 + $frames * 4)); $bw.Write([Text.Encoding]::ASCII.GetBytes('WAVE'))
  $bw.Write([Text.Encoding]::ASCII.GetBytes('fmt ')); $bw.Write([uint32]16); $bw.Write([uint16]1); $bw.Write([uint16]2); $bw.Write([uint32]48000); $bw.Write([uint32](48000 * 4)); $bw.Write([uint16]4); $bw.Write([uint16]16)
  $bw.Write([Text.Encoding]::ASCII.GetBytes('data')); $bw.Write([uint32]($frames * 4))
  for ($i = 0; $i -lt $frames; $i++) { $v = [int16]$(if ($i % 2) { 1000 } else { -1000 }); $bw.Write($v); $bw.Write($v) }
  $bw.Flush()
  [IO.File]::WriteAllBytes($wav, $ms.ToArray())
  $lv = Get-WbWavLevel -Path $wav -StartMs 100 -LengthMs 500
  Check 'a +-1000 square wave reads as RMS 1000 (-30.3 dBFS)' ([math]::Abs($lv.Rms - 1000.0) -lt 5 -and [math]::Abs($lv.Dbfs - (-30.3)) -lt 0.2)
  Check 'a level past the end of the file is unreadable, not zero' ($null -eq (Get-WbWavLevel -Path $wav -StartMs 5000 -LengthMs 500))
} finally { Remove-Item -LiteralPath $wav -ErrorAction SilentlyContinue }

# ---- The gates use it -------------------------------------------------------------------------------
foreach ($name in 'recorder-gate.ps1', 'kill-gate.ps1') {
  $t = Get-Content -LiteralPath (Join-Path $here "..\gates\$name") -Raw
  Check "$name finds the rig with Resolve-WbCableRig" ($t -match 'Resolve-WbCableRig')
  Check "$name proves it with Invoke-WbRigSelfTest before anything starts" ($t -match 'Invoke-WbRigSelfTest')
  Check "$name checks the cable is not the default playback device with the rig's names" ($t -match 'Assert-WbCableNotDefault')
  Check "$name no longer defaults to the names 'CABLE Input' / 'CABLE Output'" ($t -notmatch "PlayDevice = 'CABLE Input'" -and $t -notmatch "CaptureDevice = 'CABLE Output'")
  Check "$name keeps the player's and the capture's error text in files" ($t -match 'play\.err' -and $t -match 'reference\.err')
}
$seq = Get-Content -LiteralPath (Join-Path $here '..\gates\run-phase1-gates.ps1') -Raw
Check 'run-phase1-gates.ps1 checks the rig once before the first gate' ($seq -match 'rig-selftest\.ps1')
Check 'run-phase1-gates.ps1 stops the sequence on a refused rig' ($seq -match 'RIG REFUSED\|RIG SELF-TEST FAILED')

if ($fails) { Write-Host "$fails failed"; exit 1 }
Write-Host 'CableRig tests passed'
