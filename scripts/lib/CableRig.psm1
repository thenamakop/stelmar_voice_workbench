<#
  CableRig.psm1: the virtual cable rig of the recorder gates.

  The player (wb-testbench play) renders into one endpoint of VB-CABLE; the app and the reference
  capture record from another endpoint of the same cable. The two must belong to the SAME cable
  instance. This module finds them by the device instance behind each endpoint (the endpoint's
  parent device in Windows's device tree, for example ROOT\MEDIA\0002), never by a name or a name
  prefix: the "3-" in "CABLE Output (3- VB-Audio Virtual Cable)" is only Windows numbering
  endpoints that share a name, and the container id of these endpoints is the empty one
  ({00000000-0000-0000-FFFF-FFFFFFFFFFFF}), so it tells nothing.

  Why it exists (12 Nov 2026): after a restart the cable's render endpoint was no longer called
  "CABLE Input"; it is "Speakers (3- VB-Audio Virtual Cable)". The gates looked for a render
  device whose name contains "CABLE Input", found none, the player stopped at once (nobody read
  its error: it ran hidden), and both captures were digital silence. The same name check also
  could not see that the cable had become the default playback device.

  The rule now: refuse to start when more than one VB-CABLE instance has active endpoints, pair
  the player and the capture by instance, and before every gate play a short known signal through
  the pair and require it to arrive (found within 10 s, correlation, level).

  Refusals start with "RIG REFUSED:" and a failed signal check with "RIG SELF-TEST FAILED:";
  run-phase1-gates.ps1 stops the whole sequence on either text.
  Windows PowerShell 5.1, ASCII only.
#>
Import-Module (Join-Path $PSScriptRoot 'GateRun.psm1')

$script:MmRoot = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\MMDevices\Audio'
$script:CableProduct = 'VB-Audio Virtual Cable'

function Stop-WbRig {
  param([string]$Message)
  throw "RIG REFUSED: $Message"
}

# ---- Finding the cable's endpoints -----------------------------------------------------------------

# Every VB-CABLE audio endpoint Windows knows, active or not, with the device instance behind it.
# Source: the MMDevices registry keys (readable without administrator rights) for the state and the
# product, and the device tree (Get-PnpDeviceProperty) for the exact friendly name and the parent
# device. State 1 = active; 2 disabled; 4 not present (a stale entry from an earlier install);
# 8 unplugged.
function Get-WbCableEndpoints {
  [CmdletBinding()]
  param()
  $found = New-Object System.Collections.Generic.List[object]
  foreach ($flow in 'Render', 'Capture') {
    $prefix = $(if ($flow -eq 'Render') { '{0.0.0.00000000}' } else { '{0.0.1.00000000}' })
    foreach ($k in @(Get-ChildItem -LiteralPath "$script:MmRoot\$flow" -ErrorAction SilentlyContinue)) {
      $guid = $k.PSChildName
      $state = [int]$k.GetValue('DeviceState')
      $pk = $k.OpenSubKey('Properties')
      if ($null -eq $pk) { continue }
      try {
        $product = [string]$pk.GetValue('{b3f8fa53-0004-438e-9003-51a46e139bfc},6')
        $deviceRef = [string]$pk.GetValue('{b3f8fa53-0004-438e-9003-51a46e139bfc},2')
        $inf = [string]$pk.GetValue('{83da6326-97a6-4088-9453-a1923f573b29},3')
        $short = [string]$pk.GetValue('{a45c254e-df1c-4efd-8020-67d146a850e0},2')
        $ks = [string]$pk.GetValue('{233164c8-1b2c-4c7d-bc68-b671687a2567},1')
      } finally { $pk.Close() }
      if ($product -notlike "*$script:CableProduct*" -and $inf -notmatch 'VBCable') { continue }
      $friendly = $null
      $parent = $null
      $container = $null
      try {
        $props = @(Get-PnpDeviceProperty -InstanceId ('SWD\MMDEVAPI\' + $prefix + '.' + $guid.ToUpper()) -KeyName 'DEVPKEY_Device_FriendlyName', 'DEVPKEY_Device_Parent', 'DEVPKEY_Device_ContainerId' -ErrorAction Stop)
        foreach ($p in $props) {
          if ($null -eq $p.Data) { continue }
          if ($p.KeyName -eq 'DEVPKEY_Device_FriendlyName') { $friendly = [string]$p.Data }
          elseif ($p.KeyName -eq 'DEVPKEY_Device_Parent') { $parent = [string]$p.Data }
          elseif ($p.KeyName -eq 'DEVPKEY_Device_ContainerId') { $container = [string]$p.Data }
        }
      } catch { }
      # The registry names the same parent device ("{1}.ROOT\MEDIA\0002"): the fallback when the
      # device tree has nothing for a stale entry.
      if (-not $parent -and $deviceRef -match '^\{\d+\}\.(.+)$') { $parent = $Matches[1] }
      $found.Add([pscustomobject]@{
          Flow = $flow; Guid = $guid; EndpointId = ($prefix + '.' + $guid); State = $state; Active = ($state -eq 1)
          FriendlyName = $friendly; ShortName = $short; Instance = $parent; ContainerId = $container; KsInterface = $ks
        })
    }
  }
  @($found.ToArray() | Sort-Object Flow, Instance, FriendlyName)
}

# The VB-CABLE driver devices (Device Manager, "Sound, video and game controllers"). A restart or a
# repeated install can leave several, some of them in an error state without any endpoint.
function Get-WbCableDriverDevices {
  [CmdletBinding()]
  param()
  @(Get-PnpDevice -Class MEDIA -ErrorAction SilentlyContinue | Where-Object { $_.FriendlyName -like "*$script:CableProduct*" } | ForEach-Object {
      [pscustomobject]@{ InstanceId = [string]$_.InstanceId; Status = [string]$_.Status; Name = [string]$_.FriendlyName }
    })
}

function Format-WbCableEndpointLine {
  param($E)
  $stateName = switch ([int]$E.State) { 1 { 'active' } 2 { 'disabled' } 4 { 'not present (stale entry)' } 8 { 'unplugged' } default { "state $($E.State)" } }
  '{0,-7} {1,-26} instance {2,-16} container {3} | {4}' -f $E.Flow, $stateName, $(if ($E.Instance) { $E.Instance } else { '?' }), $(if ($E.ContainerId) { $E.ContainerId } else { '?' }), $(if ($E.FriendlyName) { $E.FriendlyName } else { "$($E.ShortName) (name unreadable)" })
}

# ---- Pairing ---------------------------------------------------------------------------------------

# A player endpoint (render) and a capture endpoint belong together only when the same cable
# instance stands behind both. Refuses otherwise, naming both.
function Assert-WbCablePair {
  [CmdletBinding()]
  param([Parameter(Mandatory = $true)] $Play, [Parameter(Mandatory = $true)] $Capture)
  if ($Play.Flow -ne 'Render') { Stop-WbRig "the player endpoint '$($Play.FriendlyName)' is not a playback (render) endpoint." }
  if ($Capture.Flow -ne 'Capture') { Stop-WbRig "the capture endpoint '$($Capture.FriendlyName)' is not a recording (capture) endpoint." }
  if (-not $Play.Instance -or -not $Capture.Instance) {
    Stop-WbRig "cannot tell which cable instance stands behind '$($Play.FriendlyName)' (instance: '$($Play.Instance)') and '$($Capture.FriendlyName)' (instance: '$($Capture.Instance)')."
  }
  if ($Play.Instance -ne $Capture.Instance) {
    Stop-WbRig "mismatched pair: the player endpoint '$($Play.FriendlyName)' belongs to cable instance $($Play.Instance), but the capture endpoint '$($Capture.FriendlyName)' belongs to cable instance $($Capture.Instance). Audio played into one cable never reaches the other. Nothing was started."
  }
}

# One active endpoint of this flow for an explicitly requested device name: the exact friendly name,
# or text that only one endpoint contains.
function Select-WbCableEndpoint {
  param($Active, [string]$Flow, [string]$Name)
  $pool = @($Active | Where-Object { $_.Flow -eq $Flow })
  $exact = @($pool | Where-Object { $_.FriendlyName -eq $Name })
  if ($exact.Count -eq 1) { return $exact[0] }
  $part = @($pool | Where-Object { $_.FriendlyName.IndexOf($Name, [StringComparison]::OrdinalIgnoreCase) -ge 0 })
  if ($part.Count -eq 1) { return $part[0] }
  $have = ($pool | ForEach-Object { "'$($_.FriendlyName)'" }) -join ', '
  if ($part.Count -gt 1) { Stop-WbRig "the $Flow device name '$Name' matches more than one active VB-CABLE endpoint ($have). Give the full name." }
  Stop-WbRig "'$Name' is not an active VB-CABLE $Flow endpoint. Active VB-CABLE $Flow endpoints: $(if ($have) { $have } else { 'none' })."
}

# Finds the cable rig: one capture endpoint and the render endpoint(s) of the same cable instance.
# -Endpoints, -Devices and -DriverDevices are for tests; normally they are read from Windows and
# from `wb-testbench devices`.
# Returns Instance, CaptureName, CaptureId, Candidates (render names in the order they are tried),
# RenderNames (every active render endpoint of the cable), PlayName (the first candidate until the
# signal check settles it) and Notes.
function Resolve-WbCableRig {
  [CmdletBinding()]
  param(
    [Parameter(Mandatory = $true)] [string]$Testbench,
    [string]$PlayDevice = '',
    [string]$CaptureDevice = '',
    $Endpoints = $null,
    $Devices = $null,
    $DriverDevices = $null
  )
  if ($null -eq $Endpoints) { $Endpoints = Get-WbCableEndpoints }
  if ($null -eq $DriverDevices) { $DriverDevices = Get-WbCableDriverDevices }
  $Endpoints = @($Endpoints)
  $active = @($Endpoints | Where-Object { $_.Active })
  if ($active.Count -eq 0) {
    Stop-WbRig 'no active VB-CABLE endpoint: install VB-CABLE (vb-audio.com), restart, and check that the cable is not disabled in Sound settings.'
  }
  $unreadable = @($active | Where-Object { -not $_.FriendlyName -or -not $_.Instance })
  if ($unreadable.Count -gt 0) {
    Stop-WbRig ("cannot read the name or the cable instance of an active VB-CABLE endpoint ({0}); the pairing cannot be checked." -f (($unreadable | ForEach-Object { "$($_.Flow) $($_.Guid)" }) -join '; '))
  }

  # More than one cable with active endpoints: the player and the captures could end up on
  # different cables. Refuse and name them.
  $instances = @($active | Group-Object Instance)
  if ($instances.Count -gt 1) {
    $parts = foreach ($g in $instances) {
      "cable instance $($g.Name): " + ((@($g.Group) | ForEach-Object { "$($_.Flow) '$($_.FriendlyName)'" }) -join '; ')
    }
    Stop-WbRig ("more than one VB-CABLE instance has active endpoints, so the player and the captures could be on different cables. {0}. Disable the endpoints of all but one instance (Sound settings, or uninstall the extra VB-CABLE driver devices in Device Manager) and run again." -f ($parts -join ' | '))
  }
  $instance = $instances[0].Name

  $caps = @($active | Where-Object { $_.Flow -eq 'Capture' })
  $rends = @($active | Where-Object { $_.Flow -eq 'Render' })
  if ($caps.Count -eq 0) { Stop-WbRig "cable instance $instance has no active capture endpoint (its render endpoints: $(($rends | ForEach-Object { "'$($_.FriendlyName)'" }) -join ', '))." }
  if ($rends.Count -eq 0) { Stop-WbRig "cable instance $instance has no active playback endpoint (its capture endpoint: $(($caps | ForEach-Object { "'$($_.FriendlyName)'" }) -join ', '))." }

  $cap = $null
  if ($CaptureDevice) { $cap = Select-WbCableEndpoint -Active $active -Flow 'Capture' -Name $CaptureDevice }
  elseif ($caps.Count -eq 1) { $cap = $caps[0] }
  else { Stop-WbRig ("cable instance {0} has more than one active capture endpoint ({1}); name the one to record from with -CaptureDevice." -f $instance, (($caps | ForEach-Object { "'$($_.FriendlyName)'" }) -join ', ')) }

  if ($PlayDevice) {
    $play = Select-WbCableEndpoint -Active $active -Flow 'Render' -Name $PlayDevice
    Assert-WbCablePair -Play $play -Capture $cap
    $candidateEndpoints = @($play)
  } else {
    foreach ($r in $rends) { Assert-WbCablePair -Play $r -Capture $cap }
    # The 16-channel endpoint last: the stereo one is what the gates are designed for.
    $candidateEndpoints = @($rends | Sort-Object @{ Expression = { $_.FriendlyName -match '16\s*ch' } }, FriendlyName)
  }

  # What wb-testbench and the app can actually see. They pick a device by a part of its name, so
  # the name must be unambiguous there, and the capture endpoint's id must be the one found above.
  if ($null -eq $Devices) {
    $res = Invoke-WbNative -FilePath $Testbench -ArgumentList @('devices')
    if ($res.ExitCode -ne 0 -or $res.Output.Count -eq 0) { Stop-WbRig "wb-testbench devices failed (exit $($res.ExitCode)): $(@($res.Error) -join ' ')" }
    $Devices = ($res.Output -join "`n") | ConvertFrom-Json
  }
  $inputs = @($Devices.inputs)
  $outputs = @($Devices.outputs)
  $capSeen = @($inputs | Where-Object { $_.name -and $_.name.IndexOf($cap.FriendlyName, [StringComparison]::OrdinalIgnoreCase) -ge 0 })
  if ($capSeen.Count -eq 0) { Stop-WbRig "the testbench and the app do not list the capture endpoint '$($cap.FriendlyName)' as a recording device." }
  if ($capSeen.Count -gt 1) { Stop-WbRig "the capture name '$($cap.FriendlyName)' is contained in more than one recording device the app can see ($(($capSeen | ForEach-Object { "'$($_.name)'" }) -join ', ')); the app and the reference capture could pick different ones." }
  if ($capSeen[0].id -and ([string]$capSeen[0].id).ToLower() -ne ('wasapi:' + $cap.EndpointId).ToLower()) {
    Stop-WbRig "the recording device the app would use for '$($cap.FriendlyName)' has id $($capSeen[0].id), not $('wasapi:' + $cap.EndpointId) (the endpoint of cable instance $instance)."
  }

  $notes = New-Object System.Collections.Generic.List[string]
  $candidates = New-Object System.Collections.Generic.List[string]
  foreach ($e in $candidateEndpoints) {
    $seen = @($outputs | Where-Object { $_ -and ([string]$_).IndexOf($e.FriendlyName, [StringComparison]::OrdinalIgnoreCase) -ge 0 })
    if ($seen.Count -eq 1) { $candidates.Add($e.FriendlyName) }
    elseif ($seen.Count -gt 1) { Stop-WbRig "the playback name '$($e.FriendlyName)' is contained in more than one playback device the player can see ($(($seen | ForEach-Object { "'$_'" }) -join ', ')); the player could pick the wrong one." }
    else { $notes.Add("playback endpoint '$($e.FriendlyName)' is active in Windows but the player cannot see it; left out") }
  }
  if ($candidates.Count -eq 0) { Stop-WbRig "none of the playback endpoints of cable instance $instance is visible to the player: $(($candidateEndpoints | ForEach-Object { "'$($_.FriendlyName)'" }) -join ', ')." }

  # Information for the evidence: stale entries and driver devices that cannot work.
  $stale = @($Endpoints | Where-Object { -not $_.Active })
  if ($stale.Count -gt 0) { $notes.Add("$($stale.Count) stale VB-CABLE endpoint entries (not present or disabled) are ignored: they are not usable devices") }
  $badDrivers = @(@($DriverDevices) | Where-Object { $_.Status -ne 'OK' })
  if ($badDrivers.Count -gt 0) { $notes.Add("VB-CABLE driver devices that are not working (no endpoints, ignored): $(($badDrivers | ForEach-Object { "$($_.InstanceId) [$($_.Status)]" }) -join ', ')") }

  [pscustomobject]@{
    Instance = $instance
    CaptureName = $cap.FriendlyName
    CaptureId = $cap.EndpointId
    Candidates = @($candidates.ToArray())
    RenderNames = @($rends | ForEach-Object { $_.FriendlyName })
    PlayName = $candidates[0]
    Notes = @($notes.ToArray())
  }
}

# ---- The signal check ------------------------------------------------------------------------------

# RMS (16-bit units) and dBFS of channel 0 of a 16-bit PCM WAV between two times.
function Get-WbWavLevel {
  [CmdletBinding()]
  param([Parameter(Mandatory = $true)] [string]$Path, [double]$StartMs = 0, [double]$LengthMs = 3000)
  $fs = [System.IO.File]::Open($Path, 'Open', 'Read', 'ReadWrite')
  try {
    $br = New-Object System.IO.BinaryReader($fs)
    [void]$br.ReadBytes(12)
    $channels = 0; $rate = 0; $bits = 0; $dataStart = -1; $dataLen = 0
    while ($fs.Position + 8 -le $fs.Length) {
      $id = [System.Text.Encoding]::ASCII.GetString($br.ReadBytes(4))
      $len = [int64]$br.ReadUInt32()
      if ($id -eq 'fmt ') {
        $fmt = $br.ReadBytes([int]$len)
        $channels = [BitConverter]::ToUInt16($fmt, 2); $rate = [BitConverter]::ToUInt32($fmt, 4); $bits = [BitConverter]::ToUInt16($fmt, 14)
      } elseif ($id -eq 'data') {
        $dataStart = $fs.Position; $dataLen = [math]::Min($len, $fs.Length - $fs.Position); break
      } else { $fs.Position += $len + ($len % 2) }
    }
    if ($dataStart -lt 0 -or $channels -lt 1 -or $bits -ne 16) { return $null }
    $block = $channels * 2
    $first = [int64]([math]::Round($StartMs * $rate / 1000.0))
    $count = [int64]([math]::Round($LengthMs * $rate / 1000.0))
    $totalFrames = [int64]($dataLen / $block)
    if ($first -ge $totalFrames) { return $null }
    if ($first + $count -gt $totalFrames) { $count = $totalFrames - $first }
    $fs.Position = $dataStart + $first * $block
    $buf = $br.ReadBytes([int]($count * $block))
    $n = [int]($buf.Length / $block)
    $sum = 0.0
    $used = 0
    for ($i = 0; $i -lt $n; $i += 2) {
      $v = [double][BitConverter]::ToInt16($buf, $i * $block)
      $sum += $v * $v; $used++
    }
    if ($used -eq 0) { return $null }
    $rms = [math]::Sqrt($sum / $used)
    [pscustomobject]@{ Rms = $rms; Dbfs = (20.0 * [math]::Log10([math]::Max($rms, 1e-9) / 32768.0)); Frames = $n }
  } finally { $fs.Dispose() }
}

# Judges one signal check from the analyser's result and the measured level. Pure (no audio).
# -MaxStartMs: the known signal must start within this long of the capture's start (10 s).
# -MinCorrelation: the analyser's own match threshold for a block (0.9).
# -MinRms: 16-bit units; the known signal's RMS is about 4,700, the analyser's silence is 200.
function Get-WbRigToneVerdict {
  [CmdletBinding()]
  param($Analysis, [string]$AnalysisError = '', $Level = $null, [double]$MaxStartMs = 10000, [double]$MinCorrelation = 0.9, [double]$MinRms = 1000)
  if ($AnalysisError -or $null -eq $Analysis) {
    $why = $(if ($AnalysisError) { $AnalysisError } else { 'no analysis was produced' })
    return [pscustomobject]@{ Passed = $false; Reason = "the known signal was not found on the capture endpoint ($why)" }
  }
  if ([double]$Analysis.content_start_ms -gt $MaxStartMs) {
    return [pscustomobject]@{ Passed = $false; Reason = ('the known signal reached the capture only after {0:n1} s (limit {1:n0} s)' -f ([double]$Analysis.content_start_ms / 1000.0), ($MaxStartMs / 1000.0)) }
  }
  if ([double]$Analysis.min_block_correlation -lt $MinCorrelation) {
    return [pscustomobject]@{ Passed = $false; Reason = ('the lowest block correlation with the known signal is {0:n3} (needs {1:n2})' -f [double]$Analysis.min_block_correlation, $MinCorrelation) }
  }
  if ($null -eq $Level -or [double]$Level.Rms -lt $MinRms) {
    return [pscustomobject]@{ Passed = $false; Reason = ('the known signal arrived too quietly (RMS {0}, needs at least {1}; the signal is about 4,700)' -f $(if ($null -eq $Level) { 'unreadable' } else { '{0:n0}' -f [double]$Level.Rms }), $MinRms) }
  }
  [pscustomobject]@{ Passed = $true; Reason = 'ok' }
}

# Plays a 6 s known signal (the gates' own seeded noise) through each candidate playback endpoint in
# turn, records it on the capture endpoint and requires the analyser to find it. The first
# candidate that passes becomes the player. Throws "RIG SELF-TEST FAILED: ..." if none does.
function Invoke-WbRigSelfTest {
  [CmdletBinding()]
  param(
    [Parameter(Mandatory = $true)] $Rig,
    [Parameter(Mandatory = $true)] [string]$Testbench,
    [Parameter(Mandatory = $true)] [string]$WorkRoot
  )
  $srcDir = Join-Path $WorkRoot 'source'
  New-Item -ItemType Directory -Force -Path $srcDir | Out-Null
  $source = Join-Path $srcDir 'rig-selftest-6s.wav'
  if (-not (Test-Path -LiteralPath ($source + '.json'))) {
    $g = Invoke-WbNative -FilePath $Testbench -ArgumentList @('gen', '--out', $source, '--seconds', '6', '--rate', '48000', '--channels', '2')
    if ($g.ExitCode -ne 0) { throw "RIG SELF-TEST FAILED: wb-testbench gen failed (exit $($g.ExitCode)): $(@($g.Error) -join ' ')" }
  }
  $stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
  $attempts = New-Object System.Collections.Generic.List[object]
  $i = 0
  foreach ($render in @($Rig.Candidates)) {
    $i++
    $dir = Join-Path $WorkRoot "rig-selftest-$stamp-$i"
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    $capDir = Join-Path $dir 'capture'
    $go = Join-Path $dir 'GO'
    $stop = Join-Path $dir 'STOP'
    $began = Get-Date
    $capProc = Start-Process -FilePath $Testbench -PassThru -WindowStyle Hidden -RedirectStandardOutput (Join-Path $dir 'capture.out') -RedirectStandardError (Join-Path $dir 'capture.err') -ArgumentList @('capture', '--device', (ConvertTo-WbArg $Rig.CaptureName), '--dir', (ConvertTo-WbArg $capDir), '--stop-file', (ConvertTo-WbArg $stop), '--go-file', (ConvertTo-WbArg $go), '--report', (ConvertTo-WbArg (Join-Path $dir 'capture.json')))
    $playProc = Start-Process -FilePath $Testbench -PassThru -WindowStyle Hidden -RedirectStandardOutput (Join-Path $dir 'play.out') -RedirectStandardError (Join-Path $dir 'play.err') -ArgumentList @('play', '--device', (ConvertTo-WbArg $render), '--file', (ConvertTo-WbArg $source), '--go-file', (ConvertTo-WbArg $go), '--report', (ConvertTo-WbArg (Join-Path $dir 'play.json')))
    # Cache the handles now: Windows PowerShell loses the exit code of a hidden process otherwise.
    $null = $capProc.Handle
    $null = $playProc.Handle
    Start-Sleep -Milliseconds 800
    Set-Content -LiteralPath $go -Value 'go' -Encoding ascii
    $playEnded = $playProc.WaitForExit(40000)
    if (-not $playEnded) { Stop-Process -Id $playProc.Id -Force -ErrorAction SilentlyContinue }
    Start-Sleep -Seconds 1
    Set-Content -LiteralPath $stop -Value 'stop' -Encoding ascii
    if (-not $capProc.WaitForExit(20000)) { Stop-Process -Id $capProc.Id -Force -ErrorAction SilentlyContinue }
    $playErr = $(if (Test-Path -LiteralPath (Join-Path $dir 'play.err')) { (Get-Content -LiteralPath (Join-Path $dir 'play.err') -Raw) } else { '' })
    $capErr = $(if (Test-Path -LiteralPath (Join-Path $dir 'capture.err')) { (Get-Content -LiteralPath (Join-Path $dir 'capture.err') -Raw) } else { '' })
    if ($null -eq $playErr) { $playErr = '' }
    if ($null -eq $capErr) { $capErr = '' }
    $playErr = $playErr.Trim()
    $capErr = $capErr.Trim()

    $analysis = $null
    $analysisErr = ''
    $level = $null
    if (-not $playEnded) {
      $analysisErr = 'the player did not finish within 40 s'
    } elseif ($playProc.ExitCode -ne 0) {
      $analysisErr = "the player stopped with exit code $($playProc.ExitCode): $playErr"
    } elseif ($capProc.ExitCode -ne 0 -and -not (Test-Path -LiteralPath (Join-Path $capDir 'parts.json'))) {
      $analysisErr = "the capture stopped with exit code $($capProc.ExitCode): $capErr"
    } else {
      $analysisErr = Invoke-WbAnalyse -Testbench $Testbench -Session $capDir -Source $source -Report (Join-Path $dir 'analysis.json')
      if (Test-Path -LiteralPath (Join-Path $dir 'analysis.json')) {
        $analysis = Get-Content -LiteralPath (Join-Path $dir 'analysis.json') -Raw | ConvertFrom-Json
      }
      if ($analysis) {
        $wav = Join-Path $capDir 'audio\part-0001.wav'
        if (Test-Path -LiteralPath $wav) { $level = Get-WbWavLevel -Path $wav -StartMs ([double]$analysis.content_start_ms) -LengthMs 4000 }
      } elseif (-not $analysisErr) { $analysisErr = 'no analysis was written' }
    }
    $verdict = Get-WbRigToneVerdict -Analysis $analysis -AnalysisError $analysisErr -Level $level
    $attempts.Add([pscustomobject]@{
        Render = $render; Passed = $verdict.Passed; Reason = $verdict.Reason; Dir = $dir
        StartMs = $(if ($analysis) { [double]$analysis.content_start_ms } else { $null })
        Correlation = $(if ($analysis) { [double]$analysis.min_block_correlation } else { $null })
        Rms = $(if ($level) { [double]$level.Rms } else { $null })
        Dbfs = $(if ($level) { [double]$level.Dbfs } else { $null })
        LeadingJunkMs = $(if ($analysis) { [double]$analysis.leading_junk_ms } else { $null })
        Seconds = ((Get-Date) - $began).TotalSeconds
      })
    if ($verdict.Passed) {
      $Rig.PlayName = $render
      return [pscustomobject]@{ Passed = $true; PlayName = $render; CaptureName = $Rig.CaptureName; Instance = $Rig.Instance; Attempts = @($attempts.ToArray()) }
    }
  }
  $why = (@($attempts.ToArray()) | ForEach-Object { "player '$($_.Render)': $($_.Reason)" }) -join ' | '
  throw ("RIG SELF-TEST FAILED: a known signal played into cable instance {0} was not heard on its capture endpoint '{1}'. {2}. No gate was started. Look at the folders {3}* under {4}." -f $Rig.Instance, $Rig.CaptureName, $why, "rig-selftest-$stamp-", $WorkRoot)
}

# The lines a gate's evidence carries about the rig.
function Get-WbCableRigLines {
  [CmdletBinding()]
  param($Rig, $SelfTest)
  if ($null -eq $Rig -or $null -eq $SelfTest) { return @('  rig: not recorded for this run (re-evaluated from saved data)') }
  $a = @($SelfTest.Attempts | Where-Object { $_.Passed } | Select-Object -First 1)
  $lines = @()
  $lines += ("  rig: the player endpoint '{0}' and the capture endpoint '{1}' both belong to VB-CABLE instance {2} (paired by device instance, not by name); the app and the reference capture record '{1}'" -f $SelfTest.PlayName, $SelfTest.CaptureName, $SelfTest.Instance)
  if ($a.Count -gt 0) {
    $lines += ("  rig self-test before the gate: a known 6 s signal played into '{0}' was found on '{1}' {2:n2} s after the capture started; lowest block correlation {3:n3}; level RMS {4:n0} ({5:n1} dBFS); the check took {6:n1} s" -f $a[0].Render, $SelfTest.CaptureName, ($a[0].StartMs / 1000.0), $a[0].Correlation, $a[0].Rms, $a[0].Dbfs, $a[0].Seconds)
  }
  $failedBefore = @($SelfTest.Attempts | Where-Object { -not $_.Passed })
  foreach ($f in $failedBefore) { $lines += ("  rig self-test, earlier candidate '{0}' did not pass: {1}" -f $f.Render, $f.Reason) }
  foreach ($n in @($Rig.Notes)) { $lines += "  rig note: $n" }
  $lines
}

# Sessions from other programs that play into the cable now (they would mix into the test signal),
# on every playback endpoint of the cable, other than the process ids in -Allow.
function Get-WbCableIntruders {
  [CmdletBinding()]
  param([Parameter(Mandatory = $true)] [string]$Testbench, [Parameter(Mandatory = $true)] [string[]]$Names, [int[]]$Allow = @())
  $seen = @{}
  $out = New-Object System.Collections.Generic.List[string]
  foreach ($n in $Names) {
    $res = Invoke-WbNative -FilePath $Testbench -ArgumentList @('sessions', '--device', $n)
    if ($res.ExitCode -ne 0 -or $res.Output.Count -eq 0) { continue }
    foreach ($s in @(($res.Output -join "`n") | ConvertFrom-Json)) {
      if (-not $s.active -or $Allow -contains [int]$s.pid) { continue }
      $key = "$($s.pid)|$($s.endpoint)"
      if ($seen.ContainsKey($key)) { continue }
      $seen[$key] = $true
      $name = (Get-Process -Id $s.pid -ErrorAction SilentlyContinue).ProcessName
      $out.Add("$name (pid $($s.pid)) on $($s.endpoint)")
    }
  }
  @($out.ToArray())
}

# Windows roles whose default playback device is the cable; refuses unless -Allow. While the cable
# is the default device every sound the computer makes (notifications, any app) plays into the
# test signal. Returns the roles (empty when the default is a real output).
function Assert-WbCableNotDefault {
  [CmdletBinding()]
  param([Parameter(Mandatory = $true)] [string]$Testbench, [Parameter(Mandatory = $true)] $Rig, [switch]$Allow, $Devices = $null)
  if ($null -eq $Devices) {
    $res = Invoke-WbNative -FilePath $Testbench -ArgumentList @('devices')
    if ($res.ExitCode -ne 0 -or $res.Output.Count -eq 0) { Stop-WbRig "wb-testbench devices failed (exit $($res.ExitCode)): $(@($res.Error) -join ' ')" }
    $Devices = ($res.Output -join "`n") | ConvertFrom-Json
  }
  $roles = @(Get-WbCableDefaultRolesFromDevices -Devices $Devices -Cable (@($Rig.RenderNames) + @('CABLE Input', 'VB-Audio Virtual Cable')))
  if ($roles.Count -gt 0 -and -not $Allow) {
    $real = @(@($Devices.outputs) | Where-Object { $_ -and $_ -notlike '*VB-Audio*' })
    Stop-WbRig ("The default playback device ({0}) is the virtual cable (cable instance {1}): every sound this computer makes would play into the test signal. Set the default playback device AND the default communications device to a real output ({2}) in Windows Settings > System > Sound (or mmsys.cpl, Playback tab), then run again (or pass -AllowCableDefault)." -f ($roles -join ', '), $Rig.Instance, $(if ($real.Count) { ($real | ForEach-Object { "'$_'" }) -join ' or ' } else { 'none found' }))
  }
  $roles
}

Export-ModuleMember -Function Assert-WbCableNotDefault, Get-WbCableEndpoints, Get-WbCableDriverDevices, Format-WbCableEndpointLine, Assert-WbCablePair, Select-WbCableEndpoint, Resolve-WbCableRig, Get-WbWavLevel, Get-WbRigToneVerdict, Invoke-WbRigSelfTest, Get-WbCableRigLines, Get-WbCableIntruders
