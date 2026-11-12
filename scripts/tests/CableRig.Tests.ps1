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

# ---- Sessions on the cable: the 12 Nov 18:10 crash --------------------------------------------------
# Windows PowerShell 5.1 returns a JSON array from ConvertFrom-Json as ONE object. With two sessions
# (the player and the always-present silent System Sounds entry, pid 0) `[int]$s.pid` threw and the
# hashing gate crashed. The parser must give one entry per session.
$two = '[{"endpoint":"Speakers (3- VB-Audio Virtual Cable)","pid":0,"active":false},{"endpoint":"Speakers (3- VB-Audio Virtual Cable)","pid":4242,"active":true}]'
$one = '[{"endpoint":"Speakers (3- VB-Audio Virtual Cable)","pid":4242,"active":true}]'
$single = '{"endpoint":"Speakers (3- VB-Audio Virtual Cable)","pid":7,"active":false}'
$ps2 = @(ConvertFrom-WbSessionJson -Text $two)
Check 'two sessions in the JSON give two entries (the 12 Nov crash)' ($ps2.Count -eq 2)
Check 'the pids are numbers, 0 and 4242' ($ps2[0].Pid -eq 0 -and $ps2[1].Pid -eq 4242 -and $ps2[0].Pid -is [int])
Check 'active flags are kept per session' (-not $ps2[0].Active -and $ps2[1].Active)
Check 'one session in an array gives one entry' (@(ConvertFrom-WbSessionJson -Text $one).Count -eq 1)
Check 'a bare object (not an array) gives one entry' (@(ConvertFrom-WbSessionJson -Text $single).Count -eq 1)
Check 'an empty array gives no entries' (@(ConvertFrom-WbSessionJson -Text '[]').Count -eq 0)
Check 'empty text gives no entries' (@(ConvertFrom-WbSessionJson -Text '').Count -eq 0)
Check 'a pid that is not a number becomes 0, not an error' ((@(ConvertFrom-WbSessionJson -Text '[{"endpoint":"x","pid":"n/a","active":true}]'))[0].Pid -eq 0)
$threw = $false; try { $null = ConvertFrom-WbSessionJson -Text 'this is not json' } catch { $threw = $true }
Check 'text that is not JSON throws in the parser (the poll catches it)' $threw
Check 'System Sounds (pid 0) is labelled by name' ((Get-WbProcessLabel -ProcessId 0).Name -match 'System Sounds')
Check 'a process id that does not exist is labelled, not an error' ((Get-WbProcessLabel -ProcessId 2147483000).Name -eq '(exited)')
Check 'this very process is labelled with its name' ((Get-WbProcessLabel -ProcessId $PID).Name -match 'powershell')

# A fake wb-testbench: prints the JSON of a session list for `sessions`. Stands in for the real one so
# the poll is tested with real parsing and no audio.
$fakeDir = Join-Path ([IO.Path]::GetTempPath()) ('wb-fake-tb-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $fakeDir | Out-Null
try {
  $jsonFile = Join-Path $fakeDir 'sessions.json'
  $fake = Join-Path $fakeDir 'fake-testbench.cmd'
  Set-Content -LiteralPath $fake -Encoding ascii -Value @('@echo off', ('type "' + $jsonFile + '"'), 'exit /b 0')
  Set-Content -LiteralPath $jsonFile -Encoding ascii -Value $two
  $names = @('Speakers (3- VB-Audio Virtual Cable)')
  $seen = @{}
  $poll = Get-WbCablePoll -Testbench $fake -Names $names -Allow @(4242) -Seen $seen
  Check 'poll with the player and System Sounds: no intruder, no error' (@($poll.Intruders).Count -eq 0 -and -not $poll.Error)
  Check 'poll records both sessions by name' ($seen.Count -eq 2)
  Check 'the player session is marked as the player, System Sounds is not' ($seen['4242|Speakers (3- VB-Audio Virtual Cable)'].IsPlayer -and -not $seen['0|Speakers (3- VB-Audio Virtual Cable)'].IsPlayer)
  $lines = @(Get-WbCableSeenLines -Seen $seen)
  Check 'evidence lines name System Sounds as silent and the player as the gate player' (($lines -join "`n") -match 'System Sounds' -and ($lines -join "`n") -match 'the gate player' -and ($lines -join "`n") -match 'silent')

  # Another program playing into the cable is found, named, and is a result, not a crash.
  Set-Content -LiteralPath $jsonFile -Encoding ascii -Value ('[{"endpoint":"Speakers (3- VB-Audio Virtual Cable)","pid":0,"active":false},{"endpoint":"Speakers (3- VB-Audio Virtual Cable)","pid":4242,"active":true},{"endpoint":"Speakers (3- VB-Audio Virtual Cable)","pid":' + $PID + ',"active":true}]')
  $seen2 = @{}
  $poll2 = Get-WbCablePoll -Testbench $fake -Names $names -Allow @(4242) -Seen $seen2
  Check 'another program playing into the cable is an intruder, named with its pid' (@($poll2.Intruders).Count -eq 1 -and $poll2.Intruders[0] -match "pid $PID")
  Check 'the intruder line carries the program name' ($poll2.Intruders[0] -match 'powershell')
  Check 'the evidence says OTHER PROGRAM PLAYING INTO THE CABLE' ((@(Get-WbCableSeenLines -Seen $seen2) -join "`n") -match 'OTHER PROGRAM PLAYING INTO THE CABLE')
  Check 'Get-WbCableIntruders (before the start) finds the same program' (@(Get-WbCableIntruders -Testbench $fake -Names $names -Allow @(4242)).Count -eq 1)
  # An active System Sounds entry (pid 0) is not the player either.
  Set-Content -LiteralPath $jsonFile -Encoding ascii -Value '[{"endpoint":"Speakers (3- VB-Audio Virtual Cable)","pid":0,"active":true},{"endpoint":"Speakers (3- VB-Audio Virtual Cable)","pid":4242,"active":true}]'
  Check 'an active System Sounds session is an intruder' (@((Get-WbCablePoll -Testbench $fake -Names $names -Allow @(4242) -Seen @{}).Intruders).Count -eq 1)

  # A parse problem or a failing testbench is reported in Error; the poll never throws.
  Set-Content -LiteralPath $jsonFile -Encoding ascii -Value 'this is not json'
  $bad = $null; $badThrew = $false
  try { $bad = Get-WbCablePoll -Testbench $fake -Names $names -Allow @(4242) -Seen @{} } catch { $badThrew = $true }
  Check 'unreadable testbench output: the poll does not throw' (-not $badThrew)
  Check 'unreadable testbench output: the problem is in Error, no intruders' ($bad -and $bad.Error -and @($bad.Intruders).Count -eq 0)
  $gone = $null; $goneThrew = $false
  try { $gone = Get-WbCablePoll -Testbench (Join-Path $fakeDir 'nothing-here.exe') -Names $names -Allow @() -Seen @{} } catch { $goneThrew = $true }
  Check 'a testbench that cannot start: the poll does not throw' ((-not $goneThrew) -and $gone -and $gone.Error)
  Check 'the pre-start intruder check does not throw either' (@(Get-WbCableIntruders -Testbench (Join-Path $fakeDir 'nothing-here.exe') -Names $names).Count -eq 0)
  $evErr = @(Get-WbCableSeenLines -Seen @{} -PollError 'boom') -join "`n"
  Check 'a poll problem is written to the evidence' ($evErr -match 'cable poll problem' -and $evErr -match 'boom')
} finally { Remove-Item -LiteralPath $fakeDir -Recurse -Force -ErrorAction SilentlyContinue }

# ---- Resident audio programs (Nahimic and the like) --------------------------------------------------
$procNahimic = @([pscustomobject]@{ ProcessName = 'NahimicSvc64'; Id = 1234; Path = 'C:\Program Files\Nahimic\NahimicSvc64.exe' })
$procFine = @([pscustomobject]@{ ProcessName = 'chrome'; Id = 1; Path = 'C:\x\chrome.exe' }, [pscustomobject]@{ ProcessName = 'RtkAudUService64'; Id = 2; Path = 'C:\x\rtk.exe' })
Check 'a running Nahimic process is found' (@(Get-WbResidentAudioPrograms -Processes $procNahimic -Services @()).Count -eq 1)
Check 'the finding names the process and its path' ((@(Get-WbResidentAudioPrograms -Processes $procNahimic -Services @()))[0] -match 'NahimicSvc64 \(pid 1234, C:\\Program Files\\Nahimic')
Check 'a running Nahimic service is found' (@(Get-WbResidentAudioPrograms -Processes @() -Services @([pscustomobject]@{ Name = 'NahimicService'; DisplayName = 'Nahimic service' })).Count -eq 1)
Check 'a browser and the Realtek helper are not resident audio programs' (@(Get-WbResidentAudioPrograms -Processes $procFine -Services @()).Count -eq 0)
Check 'A-Volute, Waves and Dolby are found too' (@(Get-WbResidentAudioPrograms -Processes @([pscustomobject]@{ ProcessName = 'AVolute'; Id = 3; Path = '' }, [pscustomobject]@{ ProcessName = 'WavesSvc64'; Id = 4; Path = '' }, [pscustomobject]@{ ProcessName = 'DolbyDAXAPI'; Id = 5; Path = '' }) -Services @()).Count -eq 3)
Check 'with one running the rig refuses, naming it and what to close' (Refuses { Assert-WbNoResidentAudio -Found @('process NahimicSvc64 (pid 1234)') } @('NahimicSvc64', 'tray', 'NahimicService', 'NahimicTask64'))
$noThrow = $true; try { Assert-WbNoResidentAudio -Found @() } catch { $noThrow = $false }
Check 'with none running the rig goes on' $noThrow

# ---- Cleanup after a gate that threw ----------------------------------------------------------------
$stopF = Join-Path ([IO.Path]::GetTempPath()) ('wb-stop-' + [guid]::NewGuid().ToString('N'))
$psExe = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
$p1 = Start-Process -FilePath $psExe -ArgumentList @('-NoProfile', '-Command', "while (-not (Test-Path '$stopF')) { Start-Sleep -Milliseconds 200 }") -PassThru -WindowStyle Hidden
$p2 = Start-Process -FilePath $psExe -ArgumentList @('-NoProfile', '-Command', 'Start-Sleep -Seconds 600') -PassThru -WindowStyle Hidden
try {
  $null = $p1.Handle; $null = $p2.Handle
  $did = @(Stop-WbRigLeftovers -Process @($p1, $p2, $null) -StopFile @($stopF) -GraceSeconds 3)
  Check 'a process that watches its stop file is stopped by it' ($p1.HasExited -and ($did -join ' ') -match "process $($p1.Id) stopped on its stop file")
  Check 'a process that ignores the stop file is ended after the grace time' ($p2.HasExited -and ($did -join ' ') -match "process $($p2.Id) did not stop")
  Check 'nothing to stop: nothing done, no error' (@(Stop-WbRigLeftovers -Process @($p1, $p2, $null) -StopFile @($stopF) -GraceSeconds 1).Count -eq 0)
} finally {
  foreach ($p in @($p1, $p2)) { try { if (-not $p.HasExited) { $p.Kill() } } catch { } }
  Remove-Item -LiteralPath $stopF -ErrorAction SilentlyContinue
}

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

foreach ($name in 'recorder-gate.ps1', 'kill-gate.ps1') {
  $g = Get-Content -LiteralPath (Join-Path $here "..\gates\$name") -Raw
  Check "$name cleans up the app, the player and the reference capture in a finally block" ($g -match 'finally \{' -and $g -match 'Stop-WbRigLeftovers')
  Check "$name polls the cable with Get-WbCablePoll (logs, never throws)" ($g -match 'Get-WbCablePoll')
  Check "$name fails (not crashes) on another program playing into the cable" ($g -match "checks\['no other program played into the cable'\]")
  Check "$name refuses to start while an audio-enhancement program runs" ($g -match 'Assert-WbNoResidentAudio')
  Check "$name writes the cable sessions it saw to the evidence" ($g -match 'Get-WbCableSeenLines')
}
$st = Get-Content -LiteralPath (Join-Path $here '..\gates\rig-selftest.ps1') -Raw
Check 'rig-selftest.ps1 refuses while an audio-enhancement program runs' ($st -match 'Assert-WbNoResidentAudio')
Check 'rig-selftest.ps1 lists the sessions on the cable and refuses when one plays' ($st -match 'Get-WbCableSessions' -and $st -match 'another program is playing into the cable')
Check 'run-phase1-gates.ps1 starts each gate with its output in a file, not a pipe' ($seq -match 'Invoke-WbWindowsPowerShellToFile' -and $seq -notmatch 'Invoke-WbWindowsPowerShell -ArgumentList')
Check 'run-phase1-gates.ps1 ends leftover processes of a gate and logs them' ($seq -match 'left a process running')

if ($fails) { Write-Host "$fails failed"; exit 1 }
Write-Host 'CableRig tests passed'
