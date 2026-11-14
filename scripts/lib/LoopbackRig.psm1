<#
  LoopbackRig.psm1: the recording rig of the laptop gate pack.

  The recorder gates feed a known signal into the app through a cable between an output and an input
  of a hardware audio interface (a loopback). This module finds that pair, proves it with a known
  signal and a level check, and returns the same rig object as CableRig.psm1 (Instance, CaptureName,
  CaptureId, Candidates, RenderNames, PlayName, Notes) plus Kind: 'loopback' or 'vb-cable'.

  If no hardware loopback is found, the rig falls back to VB-CABLE when it is installed. The fallback
  is allowed to pass its own gate but it does not close the GATES rows that ask for a hardware
  interface (76, 83, 84, 111), and every line that names the rig says so. This module never installs
  anything.

  Checks beyond CableRig's own signal check (correlation 0.9 and RMS 1000 within 10 s):
    - clipping: the loudest sample of the captured signal stays under 98 percent of full scale;
    - too hot: the signal's RMS stays under 12,000 (16-bit units; the known signal is about 4,700);
    - quiet when silent: five seconds with nothing played stay under -50 dBFS. A louder floor is a
      hum, direct monitoring feeding back, or an input that is not connected to the output.
  The settings the person must make on the interface are: line level (not microphone), low gain,
  phantom power (48 V) off, direct monitoring off. They cannot be read from Windows, so they are
  printed in the README and the refusal text, and the checks above show their effect.

  Windows PowerShell 5.1, ASCII only.
#>
Import-Module (Join-Path $PSScriptRoot 'GateRun.psm1')
Import-Module (Join-Path $PSScriptRoot 'CableRig.psm1')

$script:BuiltInPattern = 'Realtek|Smart Sound|High Definition Audio|HDMI|DisplayPort|NVIDIA|AMD|Radeon|VB-Audio|CABLE|Virtual|Stereo Mix|Remote Audio|Bluetooth|Hands-Free|Microphone Array|Array|Voicemeeter|Steam|Oculus|Wave Link'

function Get-PackDeviceName {
  param($D)
  if ($null -eq $D) { return '' }
  if ($D -is [string]) { return $D }
  if ($D.PSObject.Properties['name']) { return [string]$D.name }
  [string]$D
}

# "Line (2- Focusrite USB Audio)" -> "Focusrite USB Audio": the part that names the device, without
# the number Windows puts in front when two endpoints share a name.
function Get-PackDeviceStem {
  param([string]$Name)
  if ($Name -match '\(([^()]+)\)\s*$') { return (($Matches[1] -replace '^\s*\d+\s*-\s*', '').Trim()) }
  ''
}

# Pure. Pairs of an input and an output that belong to one outboard device, in the order they are
# tried: capture name, render names, the device stem. Built-in, HDMI and virtual devices are left out.
function Get-PackLoopbackPairs {
  [CmdletBinding()]
  param([Parameter(Mandatory = $true)] $Devices)
  $inputs = @(@($Devices.inputs) | ForEach-Object { Get-PackDeviceName $_ } | Where-Object { $_ -and $_ -notmatch $script:BuiltInPattern })
  $outputs = @(@($Devices.outputs) | ForEach-Object { Get-PackDeviceName $_ } | Where-Object { $_ -and $_ -notmatch $script:BuiltInPattern })
  $pairs = New-Object System.Collections.Generic.List[object]
  foreach ($i in $inputs) {
    $stem = Get-PackDeviceStem $i
    if (-not $stem) { continue }
    $r = @($outputs | Where-Object { (Get-PackDeviceStem $_) -eq $stem })
    if ($r.Count -gt 0) { $pairs.Add([pscustomobject]@{ Capture = $i; Renders = @($r); Stem = $stem }) }
  }
  @($pairs.ToArray())
}

# Peak (16-bit units) and RMS of channel 0 of a 16-bit PCM WAV between two times.
function Get-PackWavStats {
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
    $sum = 0.0; $peak = 0
    for ($i = 0; $i -lt $n; $i++) {
      $v = [int][BitConverter]::ToInt16($buf, $i * $block)
      $a = [math]::Abs($v)
      if ($a -gt $peak) { $peak = $a }
      $sum += [double]$v * [double]$v
    }
    if ($n -eq 0) { return $null }
    $rms = [math]::Sqrt($sum / $n)
    [pscustomobject]@{ Rms = $rms; Peak = $peak; Dbfs = (20.0 * [math]::Log10([math]::Max($rms, 1e-9) / 32768.0)); PeakDbfs = (20.0 * [math]::Log10([math]::Max([double]$peak, 1e-9) / 32768.0)); Frames = $n }
  } finally { $fs.Dispose() }
}

# Pure. Judges the levels of one rig. Signal: stats of the known signal as captured. Silence: stats of
# five seconds with nothing played. Returns Passed and Reasons.
function Get-PackRigLevelVerdict {
  [CmdletBinding()]
  param($Signal, $Silence, [double]$ClipPeak = 32112, [double]$MaxRms = 12000, [double]$MaxFloorDbfs = -50)
  $why = New-Object System.Collections.Generic.List[string]
  if ($null -eq $Signal) { $why.Add('the level of the known signal could not be read') }
  else {
    if ([double]$Signal.Peak -ge $ClipPeak) { $why.Add(('the captured signal reaches {0:n1} dBFS: it clips. Turn the interface input gain down (line level, low gain).' -f [double]$Signal.PeakDbfs)) }
    if ([double]$Signal.Rms -gt $MaxRms) { $why.Add(('the captured signal is too loud (RMS {0:n0}, limit {1:n0}). Turn the interface input gain or output level down.' -f [double]$Signal.Rms, $MaxRms)) }
  }
  if ($null -eq $Silence) { $why.Add('the noise floor with nothing played could not be read') }
  elseif ([double]$Silence.Dbfs -gt $MaxFloorDbfs) {
    $why.Add(('with nothing played the input is at {0:n1} dBFS (limit {1:n0}): a hum, direct monitoring feeding the input back, or phantom power on. Switch direct monitoring and phantom power off.' -f [double]$Silence.Dbfs, $MaxFloorDbfs))
  }
  [pscustomobject]@{ Passed = ($why.Count -eq 0); Reasons = @($why.ToArray()) }
}

function Get-PackTestbenchDevices {
  param([Parameter(Mandatory = $true)] [string]$Testbench)
  $res = Invoke-WbNative -FilePath $Testbench -ArgumentList @('devices')
  if ($res.ExitCode -ne 0 -or $res.Output.Count -eq 0) { throw "RIG REFUSED: wb-testbench devices failed (exit $($res.ExitCode)): $(@($res.Error) -join ' ')" }
  ($res.Output -join "`n") | ConvertFrom-Json
}

# Records five seconds with nothing played and returns its stats (or $null when it cannot).
function Get-PackSilenceStats {
  [CmdletBinding()]
  param([Parameter(Mandatory = $true)] [string]$Testbench, [Parameter(Mandatory = $true)] [string]$CaptureName, [Parameter(Mandatory = $true)] [string]$WorkRoot)
  $dir = Join-Path $WorkRoot ('rig-silence-' + (Get-Date -Format 'yyyyMMdd-HHmmss'))
  New-Item -ItemType Directory -Force -Path $dir | Out-Null
  $capDir = Join-Path $dir 'capture'
  $go = Join-Path $dir 'GO'
  $stop = Join-Path $dir 'STOP'
  $p = Start-Process -FilePath $Testbench -PassThru -WindowStyle Hidden -RedirectStandardOutput (Join-Path $dir 'capture.out') -RedirectStandardError (Join-Path $dir 'capture.err') -ArgumentList @('capture', '--device', (ConvertTo-WbArg $CaptureName), '--dir', (ConvertTo-WbArg $capDir), '--stop-file', (ConvertTo-WbArg $stop), '--go-file', (ConvertTo-WbArg $go), '--report', (ConvertTo-WbArg (Join-Path $dir 'capture.json')))
  $null = $p.Handle
  Start-Sleep -Milliseconds 800
  Set-Content -LiteralPath $go -Value 'go' -Encoding ascii
  Start-Sleep -Seconds 6
  Set-Content -LiteralPath $stop -Value 'stop' -Encoding ascii
  if (-not $p.WaitForExit(20000)) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue }
  $wav = Join-Path $capDir 'audio\part-0001.wav'
  if (-not (Test-Path -LiteralPath $wav)) { return $null }
  # Skip the first second (the stream start) and read the next four.
  Get-PackWavStats -Path $wav -StartMs 1000 -LengthMs 4000
}

# Finds and proves the rig. -Prefer 'loopback' tries the hardware interface first and falls back to
# VB-CABLE; 'cable' goes straight to VB-CABLE (the dry run on the Legion has no interface).
# Returns the rig object (Kind, Fallback, Banner, SelfTest, Levels added) or throws "RIG REFUSED: ...".
function Resolve-PackRig {
  [CmdletBinding()]
  param(
    [Parameter(Mandatory = $true)] [string]$Testbench,
    [Parameter(Mandatory = $true)] [string]$WorkRoot,
    [ValidateSet('loopback', 'cable')] [string]$Prefer = 'loopback',
    [string]$CaptureDevice = '',
    [string]$PlayDevice = ''
  )
  $refusals = New-Object System.Collections.Generic.List[string]
  if ($Prefer -eq 'loopback') {
    $devices = Get-PackTestbenchDevices -Testbench $Testbench
    $pairs = @(Get-PackLoopbackPairs -Devices $devices)
    if ($CaptureDevice) { $pairs = @($pairs | Where-Object { $_.Capture -like "*$CaptureDevice*" }) }
    if ($pairs.Count -eq 0) { $refusals.Add('no hardware audio interface with both an input and an output was found (built-in, HDMI and virtual devices are ignored)') }
    $tried = 0
    foreach ($pair in $pairs) {
      if ($tried -ge 4) { break }
      $tried++
      $renders = @($pair.Renders)
      if ($PlayDevice) { $renders = @($renders | Where-Object { $_ -like "*$PlayDevice*" }) }
      if ($renders.Count -eq 0) { continue }
      $rig = [pscustomobject]@{
        Instance = $pair.Stem; CaptureName = $pair.Capture; CaptureId = ''; Candidates = @($renders); RenderNames = @($renders)
        PlayName = $renders[0]; Notes = @(); Kind = 'loopback'; Fallback = $false; Banner = @(); SelfTest = $null; Levels = $null
      }
      try {
        $st = Invoke-WbRigSelfTest -Rig $rig -Testbench $Testbench -WorkRoot $WorkRoot
        $att = @($st.Attempts | Where-Object { $_.Passed })[0]
        $sig = $null
        if ($att) {
          $wav = Join-Path $att.Dir 'capture\audio\part-0001.wav'
          if (Test-Path -LiteralPath $wav) { $sig = Get-PackWavStats -Path $wav -StartMs ([double]$att.StartMs + 500) -LengthMs 4000 }
        }
        $sil = Get-PackSilenceStats -Testbench $Testbench -CaptureName $rig.CaptureName -WorkRoot $WorkRoot
        $lv = Get-PackRigLevelVerdict -Signal $sig -Silence $sil
        if (-not $lv.Passed) { $refusals.Add("interface '$($pair.Stem)' (input '$($pair.Capture)'): " + ($lv.Reasons -join ' ')); continue }
        $rig.SelfTest = $st
        $rig.Levels = [pscustomobject]@{ Signal = $sig; Silence = $sil }
        $rig.PlayName = $st.PlayName
        return $rig
      } catch {
        $refusals.Add("interface '$($pair.Stem)' (input '$($pair.Capture)'): " + $_.Exception.Message)
      }
    }
  }
  # Fallback: VB-CABLE, when it is installed. Never installed by the pack.
  $haveCable = $false
  try { $haveCable = (@(Get-WbCableEndpoints | Where-Object { $_.Active }).Count -gt 0) } catch { $haveCable = $false }
  if ($haveCable) {
    $rig = Resolve-WbCableRig -Testbench $Testbench -PlayDevice $PlayDevice -CaptureDevice $CaptureDevice
    $st = Invoke-WbRigSelfTest -Rig $rig -Testbench $Testbench -WorkRoot $WorkRoot
    Add-Member -InputObject $rig -NotePropertyName Kind -NotePropertyValue 'vb-cable' -Force
    $isFallback = ($Prefer -eq 'loopback')
    Add-Member -InputObject $rig -NotePropertyName Fallback -NotePropertyValue $isFallback -Force
    $banner = @()
    if ($isFallback) {
      $banner = @(
        '*** FALLBACK RIG: VB-CABLE, NOT A HARDWARE AUDIO INTERFACE LOOPBACK ***',
        '*** This run can pass its own gate. It does not close GATES rows 76, 83, 84 and 111. ***'
      )
      if ($refusals.Count -gt 0) { $banner += ('*** No hardware loopback because: ' + ($refusals -join ' | ') + ' ***') }
    }
    Add-Member -InputObject $rig -NotePropertyName Banner -NotePropertyValue $banner -Force
    Add-Member -InputObject $rig -NotePropertyName SelfTest -NotePropertyValue $st -Force
    Add-Member -InputObject $rig -NotePropertyName Levels -NotePropertyValue $null -Force
    return $rig
  }
  throw ('RIG REFUSED: no usable recording rig. ' + ($refusals -join ' | ') + ' Connect the audio interface with a cable from an output to an input (line level, low gain, phantom power off, direct monitoring off). VB-CABLE is not installed, and this pack does not install it.')
}

# The lines a gate's evidence carries about the rig, with the fallback notice first when it applies.
function Get-PackRigLines {
  [CmdletBinding()]
  param($Rig)
  if ($null -eq $Rig) { return @('  rig: not recorded') }
  $lines = @()
  foreach ($b in @($Rig.Banner)) { $lines += $b }
  if ($Rig.Kind -eq 'loopback') {
    $lines += ("  rig: HARDWARE LOOPBACK on the audio interface '{0}': output '{1}' cabled to input '{2}'; the app and both reference captures record '{2}'" -f $Rig.Instance, $Rig.PlayName, $Rig.CaptureName)
    if ($Rig.SelfTest) {
      $a = @($Rig.SelfTest.Attempts | Where-Object { $_.Passed } | Select-Object -First 1)
      if ($a.Count -gt 0) { $lines += ("  rig self-test: a known 6 s signal was found {0:n2} s after the capture started; lowest block correlation {1:n3}; level RMS {2:n0} ({3:n1} dBFS)" -f ($a[0].StartMs / 1000.0), $a[0].Correlation, $a[0].Rms, $a[0].Dbfs) }
    }
    if ($Rig.Levels) {
      if ($Rig.Levels.Signal) { $lines += ("  rig level: peak {0:n1} dBFS, RMS {1:n1} dBFS (clipping above -0.2 dBFS)" -f $Rig.Levels.Signal.PeakDbfs, $Rig.Levels.Signal.Dbfs) }
      if ($Rig.Levels.Silence) { $lines += ("  rig noise floor with nothing played: {0:n1} dBFS (limit -50 dBFS)" -f $Rig.Levels.Silence.Dbfs) }
    }
    foreach ($n in @($Rig.Notes)) { $lines += "  rig note: $n" }
  } else {
    $lines += Get-WbCableRigLines -Rig $Rig -SelfTest $Rig.SelfTest
  }
  $lines
}

# Short text for a result file: which rig, and whether the run may close the hardware-interface rows.
function Get-PackRigLabel {
  param($Rig)
  if ($null -eq $Rig) { return 'none' }
  if ($Rig.Kind -eq 'loopback') { return "hardware loopback ($($Rig.Instance))" }
  if ($Rig.Fallback) { return 'VB-CABLE FALLBACK (does not close rows 76, 83, 84, 111)' }
  'VB-CABLE'
}

Export-ModuleMember -Function Get-PackDeviceName, Get-PackDeviceStem, Get-PackLoopbackPairs, Get-PackWavStats, Get-PackRigLevelVerdict, Get-PackTestbenchDevices, Get-PackSilenceStats, Resolve-PackRig, Get-PackRigLines, Get-PackRigLabel
