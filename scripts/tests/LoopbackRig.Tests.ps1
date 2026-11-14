# Tests for scripts/lib/LoopbackRig.psm1: pairing an interface's input and output, the level verdict,
# and the WAV level reader. No audio device is touched.
# Run: powershell -NoProfile -ExecutionPolicy Bypass -File scripts/tests/LoopbackRig.Tests.ps1
# ASCII only; Windows PowerShell 5.1.
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
Import-Module (Join-Path $here '..\lib\LoopbackRig.psm1') -Force
$fails = 0
function Check([string]$Name, [bool]$Ok) {
  if ($Ok) { Write-Host "ok   $Name" } else { Write-Host "FAIL $Name"; $script:fails++ }
}

# ---- stems and pairs ----
Check 'the stem is the last bracket' ((Get-PackDeviceStem 'Line (Focusrite USB Audio)') -eq 'Focusrite USB Audio')
Check 'the Windows numbering is removed from the stem' ((Get-PackDeviceStem 'Speakers (2- Focusrite USB Audio)') -eq 'Focusrite USB Audio')
Check 'a name without a bracket has no stem' ((Get-PackDeviceStem 'Default') -eq '')

$devs = [pscustomobject]@{
  inputs = @(
    [pscustomobject]@{ name = 'Microphone Array (Realtek(R) Audio)'; id = 'a' },
    [pscustomobject]@{ name = 'Line (2- Focusrite USB Audio)'; id = 'b' },
    [pscustomobject]@{ name = 'CABLE Output (VB-Audio Virtual Cable)'; id = 'c' },
    [pscustomobject]@{ name = 'Stereo Mix (Realtek(R) Audio)'; id = 'd' })
  outputs = @('Speakers (Realtek(R) Audio)', 'Speakers (2- Focusrite USB Audio)', 'Headphones (Focusrite USB Audio)', 'CABLE Input (VB-Audio Virtual Cable)', 'LG HDR (NVIDIA High Definition Audio)')
}
$pairs = @(Get-PackLoopbackPairs -Devices $devs)
Check 'one interface pair is found among built-in and virtual devices' ($pairs.Count -eq 1)
Check 'the pair has the capture and both renders' ($pairs[0].Capture -eq 'Line (2- Focusrite USB Audio)' -and @($pairs[0].Renders).Count -eq 2)
$none = [pscustomobject]@{ inputs = @([pscustomobject]@{ name = 'Microphone Array (Realtek(R) Audio)' }); outputs = @('Speakers (Realtek(R) Audio)') }
Check 'a laptop with only built-in audio has no pair' (@(Get-PackLoopbackPairs -Devices $none).Count -eq 0)
$two = [pscustomobject]@{
  inputs = @([pscustomobject]@{ name = 'Line 1-2 (UMC204HD 192k)' }, [pscustomobject]@{ name = 'Line 3-4 (UMC204HD 192k)' })
  outputs = @('Main (UMC204HD 192k)')
}
Check 'two inputs of one interface give two pairs' (@(Get-PackLoopbackPairs -Devices $two).Count -eq 2)

# ---- level verdict ----
$good = [pscustomobject]@{ Rms = 4700; Peak = 16000; Dbfs = -16.9; PeakDbfs = -6.2; Frames = 1 }
$floor = [pscustomobject]@{ Rms = 8; Peak = 40; Dbfs = -72.2; PeakDbfs = -58; Frames = 1 }
Check 'a clean rig passes' ((Get-PackRigLevelVerdict -Signal $good -Silence $floor).Passed)
$clip = [pscustomobject]@{ Rms = 9000; Peak = 32700; Dbfs = -11; PeakDbfs = -0.05; Frames = 1 }
$v = Get-PackRigLevelVerdict -Signal $clip -Silence $floor
Check 'a clipping signal fails and says so' (-not $v.Passed -and ($v.Reasons -join ' ') -match 'clips')
$hot = [pscustomobject]@{ Rms = 13000; Peak = 30000; Dbfs = -8; PeakDbfs = -0.7; Frames = 1 }
Check 'a signal that is too loud fails' (-not (Get-PackRigLevelVerdict -Signal $hot -Silence $floor).Passed)
$hum = [pscustomobject]@{ Rms = 800; Peak = 3000; Dbfs = -32; PeakDbfs = -20; Frames = 1 }
$v = Get-PackRigLevelVerdict -Signal $good -Silence $hum
Check 'a noisy floor fails and names monitoring and phantom power' (-not $v.Passed -and ($v.Reasons -join ' ') -match 'monitoring')
Check 'an unreadable level fails' (-not (Get-PackRigLevelVerdict -Signal $null -Silence $floor).Passed -and -not (Get-PackRigLevelVerdict -Signal $good -Silence $null).Passed)

# ---- WAV reader ----
$tmp = Join-Path ([IO.Path]::GetTempPath()) ('wb-lb-' + [guid]::NewGuid().ToString('N') + '.wav')
try {
  $rate = 48000; $frames = $rate * 2
  $ms = New-Object System.IO.MemoryStream
  $bw = New-Object System.IO.BinaryWriter($ms)
  $bw.Write([Text.Encoding]::ASCII.GetBytes('RIFF')); $bw.Write([uint32](36 + $frames * 4)); $bw.Write([Text.Encoding]::ASCII.GetBytes('WAVE'))
  $bw.Write([Text.Encoding]::ASCII.GetBytes('fmt ')); $bw.Write([uint32]16); $bw.Write([uint16]1); $bw.Write([uint16]2); $bw.Write([uint32]$rate); $bw.Write([uint32]($rate * 4)); $bw.Write([uint16]4); $bw.Write([uint16]16)
  $bw.Write([Text.Encoding]::ASCII.GetBytes('data')); $bw.Write([uint32]($frames * 4))
  for ($i = 0; $i -lt $frames; $i++) {
    $a = $(if ($i -lt $rate) { 0 } else { [int16](10000 * [math]::Sin(2 * [math]::PI * 440 * $i / $rate)) })
    $bw.Write([int16]$a); $bw.Write([int16]$a)
  }
  $bw.Flush()
  [IO.File]::WriteAllBytes($tmp, $ms.ToArray())
  $s0 = Get-PackWavStats -Path $tmp -StartMs 0 -LengthMs 900
  Check 'a silent stretch reads as zero' ($s0.Peak -eq 0 -and $s0.Rms -eq 0)
  $s1 = Get-PackWavStats -Path $tmp -StartMs 1100 -LengthMs 500
  Check 'a 10000 sine reads RMS near 7071 and peak near 10000' ([math]::Abs($s1.Rms - 7071) -lt 120 -and $s1.Peak -ge 9900 -and $s1.Peak -le 10000)
  Check 'a start past the end reads as null' ($null -eq (Get-PackWavStats -Path $tmp -StartMs 9000 -LengthMs 100))
} finally { Remove-Item -LiteralPath $tmp -ErrorAction SilentlyContinue }

# ---- labels ----
$lb = [pscustomobject]@{ Kind = 'loopback'; Instance = 'Focusrite USB Audio'; Fallback = $false }
$fb = [pscustomobject]@{ Kind = 'vb-cable'; Instance = 'x'; Fallback = $true }
Check 'the label of a hardware rig names the interface' ((Get-PackRigLabel $lb) -match 'Focusrite')
Check 'the label of a fallback says it does not close the rows' ((Get-PackRigLabel $fb) -match 'FALLBACK' -and (Get-PackRigLabel $fb) -match '76')
$fbLines = @(Get-PackRigLines -Rig ([pscustomobject]@{ Kind = 'vb-cable'; Banner = @('*** FALLBACK RIG ***'); SelfTest = $null; Notes = @(); Instance = 'x' }))
Check 'the evidence lines of a fallback start with the banner' ($fbLines[0] -match 'FALLBACK')

if ($fails) { Write-Host "$fails failed"; exit 1 }
Write-Host 'LoopbackRig tests passed'
