# Tests for scripts/lib/PackBuild.psm1: WAV slice, forbidden-content scan, manifest check, exe strings, size report.
# Run: powershell -NoProfile -ExecutionPolicy Bypass -File scripts/tests/PackBuild.Tests.ps1
# ASCII only; Windows PowerShell 5.1.
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
Import-Module (Join-Path $here '..\lib\PackBuild.psm1') -Force
$fails = 0
function Check([string]$Name, [bool]$Ok) {
  if ($Ok) { Write-Host "ok   $Name" } else { Write-Host "FAIL $Name"; $script:fails++ }
}
$tmp = Join-Path ([IO.Path]::GetTempPath()) ('wb-pb-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path $tmp | Out-Null
try {
  # --- WAV slice: 10 s of a ramp at 16 kHz mono ---
  $rate = 16000
  $n = $rate * 10
  $data = New-Object byte[] ($n * 2)
  for ($i = 0; $i -lt $n; $i++) { [BitConverter]::GetBytes([int16]($i % 30000)).CopyTo($data, $i * 2) }
  $hdr = New-Object byte[] 44
  [Text.Encoding]::ASCII.GetBytes('RIFF').CopyTo($hdr, 0); [BitConverter]::GetBytes([uint32](36 + $data.Length)).CopyTo($hdr, 4)
  [Text.Encoding]::ASCII.GetBytes('WAVEfmt ').CopyTo($hdr, 8); [BitConverter]::GetBytes([uint32]16).CopyTo($hdr, 16)
  [BitConverter]::GetBytes([uint16]1).CopyTo($hdr, 20); [BitConverter]::GetBytes([uint16]1).CopyTo($hdr, 22)
  [BitConverter]::GetBytes([uint32]$rate).CopyTo($hdr, 24); [BitConverter]::GetBytes([uint32]($rate * 2)).CopyTo($hdr, 28)
  [BitConverter]::GetBytes([uint16]2).CopyTo($hdr, 32); [BitConverter]::GetBytes([uint16]16).CopyTo($hdr, 34)
  [Text.Encoding]::ASCII.GetBytes('data').CopyTo($hdr, 36); [BitConverter]::GetBytes([uint32]$data.Length).CopyTo($hdr, 40)
  $src = Join-Path $tmp 'src.wav'
  [IO.File]::WriteAllBytes($src, ($hdr + $data))
  $dst = Join-Path $tmp 'slice.wav'
  $m = New-PackWavSlice -Source $src -Dest $dst -StartSeconds 2 -Seconds 3
  $o = [IO.File]::ReadAllBytes($dst)
  Check 'the slice is 3 s of 16 kHz mono 16-bit' ($m.duration_s -eq 3 -and $m.rate -eq 16000 -and $m.channels -eq 1 -and $o.Length -eq 44 + 3 * 16000 * 2)
  Check 'the slice starts at sample 32000 of the source' ([BitConverter]::ToInt16($o, 44) -eq [int16](32000 % 30000))
  Check 'the slice header says how long the data is' ([BitConverter]::ToUInt32($o, 40) -eq 96000)
  function Throws([scriptblock]$Sb) { try { & $Sb | Out-Null; $false } catch { $true } }
  Check 'a slice past the end is an error' (Throws { New-PackWavSlice -Source $src -Dest $dst -StartSeconds 20 -Seconds 1 })
  Set-Content -LiteralPath (Join-Path $tmp 'junk.wav') -Value 'not a wav' -Encoding ascii
  Check 'a file that is not a WAV is an error' (Throws { New-PackWavSlice -Source (Join-Path $tmp 'junk.wav') -Dest $dst })

  # --- forbidden content ---
  $pack = Join-Path $tmp 'pack'
  New-Item -ItemType Directory -Force -Path (Join-Path $pack 'scripts\lib'), (Join-Path $pack 'resources\bench'), (Join-Path $pack 'target\release'), (Join-Path $pack 'work') | Out-Null
  Set-Content -LiteralPath (Join-Path $pack 'scripts\lib\ok.psm1') -Value 'function A { 1 }' -Encoding ascii
  Set-Content -LiteralPath (Join-Path $pack 'resources\bench\bench-en.wav') -Value 'x' -Encoding ascii
  $clean = @(Get-PackForbidden -Root $pack -AllowedWav @('resources/bench/bench-en.wav'))
  Check 'a clean pack has no findings' ($clean.Count -eq 0)
  Set-Content -LiteralPath (Join-Path $pack 'main.rs') -Value 'fn main() {}' -Encoding ascii
  Set-Content -LiteralPath (Join-Path $pack 'target\release\wb-emulate.exe') -Value 'x' -Encoding ascii
  Set-Content -LiteralPath (Join-Path $pack 'target\release\cargo.exe') -Value 'x' -Encoding ascii
  Set-Content -LiteralPath (Join-Path $pack 'resources\bench\private.wav') -Value 'x' -Encoding ascii
  Set-Content -LiteralPath (Join-Path $pack 'scripts\lib\leak.ps1') -Value ('$t = ' + "'hf_" + ('a' * 34) + "'") -Encoding ascii
  Set-Content -LiteralPath (Join-Path $pack 'work\anything.rs') -Value 'ignored' -Encoding ascii
  $f = @(Get-PackForbidden -Root $pack -AllowedWav @('resources/bench/bench-en.wav'))
  Check 'source code is found' (@($f | Where-Object { $_.Kind -eq 'source' -and $_.Path -eq 'main.rs' }).Count -eq 1)
  Check 'wb-emulate is found as the emulator' (@($f | Where-Object { $_.Kind -eq 'emulator' }).Count -eq 1)
  Check 'cargo is found as a tool' (@($f | Where-Object { $_.Kind -eq 'tool' }).Count -eq 1)
  Check 'a WAV off the allow-list is found as audio and the listed one is not' (@($f | Where-Object { $_.Kind -eq 'audio' }).Count -eq 1 -and $f.Path -notcontains 'resources\bench\bench-en.wav')
  Check 'a token-looking string is found' (@($f | Where-Object { $_.Kind -eq 'secret' -and $_.Path -like '*leak.ps1' }).Count -eq 1)
  Check 'the work folder is not scanned' (@($f | Where-Object { $_.Path -like 'work*' }).Count -eq 0)

  # --- manifest ---
  $m1 = Join-Path $tmp 'p2'
  New-Item -ItemType Directory -Force -Path (Join-Path $m1 'a') | Out-Null
  Set-Content -LiteralPath (Join-Path $m1 'a\one.txt') -Value 'one' -Encoding ascii
  Set-Content -LiteralPath (Join-Path $m1 'two.txt') -Value 'two' -Encoding ascii
  $files = @('a/one.txt', 'two.txt') | ForEach-Object { [ordered]@{ path = $_; bytes = 1; sha256 = (Get-FileHash -LiteralPath (Join-Path $m1 ($_ -replace '/', '\')) -Algorithm SHA256).Hash.ToLowerInvariant() } }
  ([ordered]@{ source_commit = 'x'; built_at = 'now'; files = @($files) } | ConvertTo-Json -Depth 5) | Set-Content -LiteralPath (Join-Path $m1 'PACK-MANIFEST.json') -Encoding UTF8
  Check 'a pack that matches its manifest has no problems' (@(Test-PackManifest -Root $m1).Count -eq 0)
  Set-Content -LiteralPath (Join-Path $m1 'two.txt') -Value 'changed' -Encoding ascii
  Set-Content -LiteralPath (Join-Path $m1 'extra.txt') -Value 'extra' -Encoding ascii
  Remove-Item -LiteralPath (Join-Path $m1 'a\one.txt')
  $pr = @(Test-PackManifest -Root $m1)
  Check 'a changed, a missing and an unlisted file are each reported' ($pr -contains 'changed: two.txt' -and $pr -contains 'missing: a/one.txt' -and $pr -contains 'not in the manifest: extra.txt')

  # --- exe strings ---
  $exe = Join-Path $tmp 'fake.exe'
  $bytes = [Text.Encoding]::ASCII.GetBytes('xxxx') + [Text.Encoding]::Unicode.GetBytes('WB_GATE_CHECK') + [Text.Encoding]::ASCII.GetBytes('yyyy')
  [IO.File]::WriteAllBytes($exe, $bytes)
  Check 'a UTF-16 string is found in a binary' (Test-PackExeStrings -Path $exe -Text 'WB_GATE_')
  Check 'an absent string is not found' (-not (Test-PackExeStrings -Path $exe -Text 'WB_NOTHING'))
  [IO.File]::WriteAllBytes($exe, [Text.Encoding]::ASCII.GetBytes('a WB_GATE_X b'))
  Check 'an ASCII string is found in a binary' (Test-PackExeStrings -Path $exe -Text 'WB_GATE_')

  # --- size report ---
  $rep = @(Format-PackSizeReport -Entries @([pscustomobject]@{ Path = 'models/a.bin'; Bytes = 5GB }, [pscustomobject]@{ Path = 'run-gates.ps1'; Bytes = 1MB }) -UsbGB @(4, 8))
  Check 'the report names the total and the largest file' ($rep[0] -match '5\.\d\d GB in 2 files' -and ($rep -join "`n") -match 'largest file: models/a\.bin' -and ($rep -join "`n") -match 'does not fit FAT32')
  Check 'a 4 GB drive does not hold 5 GB and an 8 GB drive does' (($rep -join "`n") -match '4 GB USB drive[^\n]*does not fit' -and ($rep -join "`n") -match '8 GB USB drive[^\n]*fits')
} finally { Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue }
if ($fails) { Write-Host "$fails failed"; exit 1 }
Write-Host 'PackBuild tests passed'
