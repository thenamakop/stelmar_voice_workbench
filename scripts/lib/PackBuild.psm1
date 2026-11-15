<#
  PackBuild.psm1: what build-gate-pack.ps1 and check-gate-pack.ps1 share. Development side only: this
  module is never copied into the pack.

    New-PackWavSlice      cut a few seconds out of a 16-bit PCM WAV file (the network-off test clip)
    Get-PackForbidden     what must not be in the pack: source, development tools, wb-emulate, audio that
                          is not on the allow-list, anything that looks like a secret
    Test-PackManifest     every file of PACK-MANIFEST.json is present with the same SHA-256, nothing else is
    Test-PackExeStrings   a string (ASCII or UTF-16) occurs in an executable: the gate hooks must not be
                          in the release build
    Format-PackSizeReport the pack's size against the USB drives people own

  Windows PowerShell 5.1, ASCII only.
#>

Set-StrictMode -Version 2

# Cuts -Seconds seconds starting at -StartSeconds out of a 16-bit PCM WAV and writes a new WAV file.
function New-PackWavSlice {
  [CmdletBinding()]
  param([Parameter(Mandatory = $true)] [string]$Source, [Parameter(Mandatory = $true)] [string]$Dest, [double]$StartSeconds = 6, [double]$Seconds = 6)
  $b = [System.IO.File]::ReadAllBytes($Source)
  if ($b.Length -lt 44 -or [Text.Encoding]::ASCII.GetString($b, 0, 4) -ne 'RIFF' -or [Text.Encoding]::ASCII.GetString($b, 8, 4) -ne 'WAVE') { throw "$Source is not a WAV file" }
  $pos = 12
  $fmt = $null
  $dataAt = -1
  $dataLen = 0
  while ($pos + 8 -le $b.Length) {
    $id = [Text.Encoding]::ASCII.GetString($b, $pos, 4)
    $len = [int][BitConverter]::ToUInt32($b, $pos + 4)
    if ($id -eq 'fmt ') { $fmt = $pos + 8 }
    if ($id -eq 'data') { $dataAt = $pos + 8; $dataLen = [math]::Min($len, $b.Length - $dataAt); break }
    $pos += 8 + $len + ($len % 2)
  }
  if ($null -eq $fmt -or $dataAt -lt 0) { throw "$Source has no fmt or data chunk" }
  $tag = [BitConverter]::ToUInt16($b, $fmt)
  $channels = [int][BitConverter]::ToUInt16($b, $fmt + 2)
  $rate = [int][BitConverter]::ToUInt32($b, $fmt + 4)
  $bits = [int][BitConverter]::ToUInt16($b, $fmt + 14)
  if ($tag -ne 1 -or $bits -ne 16) { throw "$Source is not 16-bit PCM" }
  $frame = $channels * 2
  $from = [int][math]::Floor($StartSeconds * $rate) * $frame
  $count = [int][math]::Floor($Seconds * $rate) * $frame
  if ($from -ge $dataLen) { throw 'the slice starts after the end of the audio' }
  $count = [math]::Min($count, $dataLen - $from)
  $out = New-Object byte[] (44 + $count)
  [Text.Encoding]::ASCII.GetBytes('RIFF').CopyTo($out, 0)
  [BitConverter]::GetBytes([uint32](36 + $count)).CopyTo($out, 4)
  [Text.Encoding]::ASCII.GetBytes('WAVEfmt ').CopyTo($out, 8)
  [BitConverter]::GetBytes([uint32]16).CopyTo($out, 16)
  [BitConverter]::GetBytes([uint16]1).CopyTo($out, 20)
  [BitConverter]::GetBytes([uint16]$channels).CopyTo($out, 22)
  [BitConverter]::GetBytes([uint32]$rate).CopyTo($out, 24)
  [BitConverter]::GetBytes([uint32]($rate * $frame)).CopyTo($out, 28)
  [BitConverter]::GetBytes([uint16]$frame).CopyTo($out, 32)
  [BitConverter]::GetBytes([uint16]16).CopyTo($out, 34)
  [Text.Encoding]::ASCII.GetBytes('data').CopyTo($out, 36)
  [BitConverter]::GetBytes([uint32]$count).CopyTo($out, 40)
  [Array]::Copy($b, $dataAt + $from, $out, 44, $count)
  [System.IO.File]::WriteAllBytes($Dest, $out)
  [pscustomobject]@{ duration_s = [math]::Round($count / $frame / [double]$rate, 3); rate = $rate; channels = $channels; bits = $bits }
}

# The start (seconds) of the window of the given length that holds the most speech: the loudest 100 ms frames
# (above 5% of the loudest frame) counted in every window, stepped by half a second. A fixed start can land in
# a pause: the network-off gate needs a clip the engines really decode.
function Find-PackSpeechStart {
  [CmdletBinding()]
  param([Parameter(Mandatory = $true)] [string]$Source, [double]$Seconds = 6, [double]$StepSeconds = 0.5)
  $b = [System.IO.File]::ReadAllBytes($Source)
  $pos = 12; $fmt = $null; $dataAt = -1; $dataLen = 0
  while ($pos + 8 -le $b.Length) {
    $id = [Text.Encoding]::ASCII.GetString($b, $pos, 4)
    $len = [int][BitConverter]::ToUInt32($b, $pos + 4)
    if ($id -eq 'fmt ') { $fmt = $pos + 8 }
    if ($id -eq 'data') { $dataAt = $pos + 8; $dataLen = [math]::Min($len, $b.Length - $dataAt); break }
    $pos += 8 + $len + ($len % 2)
  }
  if ($null -eq $fmt -or $dataAt -lt 0) { throw "$Source has no fmt or data chunk" }
  $channels = [int][BitConverter]::ToUInt16($b, $fmt + 2)
  $rate = [int][BitConverter]::ToUInt32($b, $fmt + 4)
  $frameBytes = $channels * 2
  $samplesPerFrame = [int]($rate / 10)
  $nFrames = [int][math]::Floor(($dataLen / $frameBytes) / $samplesPerFrame)
  $rms = New-Object double[] $nFrames
  for ($f = 0; $f -lt $nFrames; $f++) {
    $sum = 0.0
    $base = $dataAt + $f * $samplesPerFrame * $frameBytes
    for ($i = 0; $i -lt $samplesPerFrame; $i++) { $v = [double][BitConverter]::ToInt16($b, $base + $i * $frameBytes); $sum += $v * $v }
    $rms[$f] = [math]::Sqrt($sum / $samplesPerFrame)
  }
  $max = 0.0; foreach ($x in $rms) { if ($x -gt $max) { $max = $x } }
  if ($max -le 0) { return 0.0 }
  $thr = 0.05 * $max
  $winFrames = [int][math]::Round($Seconds * 10)
  $best = 0.0; $bestScore = -1
  $total = $nFrames / 10.0
  for ($s = 0.0; $s + $Seconds -le $total + 1e-9; $s += $StepSeconds) {
    $from = [int][math]::Round($s * 10); $score = 0
    for ($f = $from; $f -lt $from + $winFrames -and $f -lt $nFrames; $f++) { if ($rms[$f] -gt $thr) { $score++ } }
    if ($score -gt $bestScore) { $bestScore = $score; $best = $s }
  }
  $best
}

$script:SourceExt = @('.rs', '.py', '.ts', '.tsx', '.jsx', '.js', '.mjs', '.cjs', '.toml', '.c', '.cc', '.cpp', '.h', '.hpp', '.cs', '.java', '.go', '.lock', '.pdb', '.rlib', '.rmeta', '.obj', '.ipynb')
$script:SourceNames = @('package.json', 'pnpm-lock.yaml', 'package-lock.json', 'yarn.lock', 'tsconfig.json', 'cargo.toml', 'cargo.lock', 'rust-toolchain.toml', '.gitignore', '.gitattributes')
$script:ToolNames = @('cargo.exe', 'rustc.exe', 'rustup.exe', 'rustfmt.exe', 'clippy-driver.exe', 'node.exe', 'npm.cmd', 'npx.cmd', 'pnpm.cmd', 'pnpm.exe', 'python.exe', 'pythonw.exe', 'py.exe', 'pip.exe', 'git.exe', 'dumpbin.exe', 'cl.exe', 'link.exe', 'cmake.exe', 'ninja.exe', 'nvcc.exe', 'msbuild.exe', 'whisper-cli.exe', 'ffmpeg.exe', 'wb-emulate.exe', 'mock_engine.exe')
$script:TextExt = @('.ps1', '.psm1', '.psd1', '.json', '.txt', '.md', '.csv', '.log', '.cmd', '.bat', '.xml', '.html', '.css', '.yml', '.yaml', '.jsonl')
$script:SecretPatterns = @(
  @{ Name = 'a Hugging Face token'; Regex = 'hf_[A-Za-z0-9]{30,}' },
  @{ Name = 'a GitHub token'; Regex = '(ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{30,}' },
  @{ Name = 'an API key'; Regex = 'sk-ant-[A-Za-z0-9_\-]{20,}|AKIA[0-9A-Z]{16}|xox[bp]-[0-9A-Za-z\-]{20,}' },
  @{ Name = 'a private key'; Regex = '-----BEGIN (RSA |EC |OPENSSH |)PRIVATE KEY-----' },
  @{ Name = 'a bearer token'; Regex = '(?i)authorization:\s*bearer\s+[A-Za-z0-9._\-]{20,}' }
)

# Pure. The secret patterns the check looks for (names), so a test can see them.
function Get-PackSecretPatternNames { @($script:SecretPatterns | ForEach-Object { $_.Name }) }

# Walks -Root and returns one finding per problem: Kind (source, tool, emulator, audio, secret, git),
# Path (relative) and Why. -AllowedWav lists the relative paths (lower case, forward slashes) of the WAV
# files that may be in the pack (public clips only).
function Get-PackForbidden {
  [CmdletBinding()]
  param([Parameter(Mandatory = $true)] [string]$Root, [string[]]$AllowedWav = @())
  $rootFull = (Resolve-Path -LiteralPath $Root).Path.TrimEnd('\')
  $allowed = @{}
  foreach ($a in $AllowedWav) { $allowed[([string]$a).ToLowerInvariant().Replace('\', '/')] = $true }
  $out = New-Object System.Collections.Generic.List[object]
  foreach ($d in @(Get-ChildItem -LiteralPath $rootFull -Recurse -Directory -Force -ErrorAction SilentlyContinue)) {
    if ($d.Name -in @('.git', 'node_modules', '.venv', '__pycache__', '.cargo', '.rustup')) {
      $out.Add([pscustomobject]@{ Kind = 'git'; Path = $d.FullName.Substring($rootFull.Length + 1); Why = 'a development folder' })
    }
  }
  foreach ($f in @(Get-ChildItem -LiteralPath $rootFull -Recurse -File -Force -ErrorAction SilentlyContinue)) {
    $rel = $f.FullName.Substring($rootFull.Length + 1)
    $relKey = $rel.ToLowerInvariant().Replace('\', '/')
    $name = $f.Name.ToLowerInvariant()
    $ext = $f.Extension.ToLowerInvariant()
    if ($relKey -like 'work/*' -or $relKey -like 'gate-results-*.zip') { continue }
    if ($script:ToolNames -contains $name) {
      $kind = $(if ($name -like 'wb-emulate*') { 'emulator' } else { 'tool' })
      $out.Add([pscustomobject]@{ Kind = $kind; Path = $rel; Why = $(if ($kind -eq 'emulator') { 'wb-emulate must stay out of every bundle' } else { 'a development tool' }) })
      continue
    }
    if ($name -like 'wb-emulate*') { $out.Add([pscustomobject]@{ Kind = 'emulator'; Path = $rel; Why = 'wb-emulate must stay out of every bundle' }); continue }
    if (($script:SourceExt -contains $ext) -or ($script:SourceNames -contains $name)) {
      $out.Add([pscustomobject]@{ Kind = 'source'; Path = $rel; Why = 'source code or a build file' })
      continue
    }
    if ($ext -eq '.wav' -and -not $allowed.ContainsKey($relKey)) {
      $out.Add([pscustomobject]@{ Kind = 'audio'; Path = $rel; Why = 'a WAV file that is not on the public-clip allow-list' })
      continue
    }
    if (($name -like '.env*') -or ($name -in @('hf_token', 'token.txt', 'credentials.json', 'id_rsa', 'id_ed25519')) -or ($ext -in @('.pem', '.key', '.pfx', '.p12'))) {
      $out.Add([pscustomobject]@{ Kind = 'secret'; Path = $rel; Why = 'a file named like a secret' })
      continue
    }
    if ($script:TextExt -contains $ext -and $f.Length -le 8MB) {
      $text = [System.IO.File]::ReadAllText($f.FullName)
      foreach ($p in $script:SecretPatterns) {
        if ([regex]::IsMatch($text, $p.Regex)) { $out.Add([pscustomobject]@{ Kind = 'secret'; Path = $rel; Why = ('looks like ' + $p.Name) }) }
      }
    }
  }
  @($out.ToArray())
}

# Compares PACK-MANIFEST.json with the folder: problems are missing files, changed files and files the
# manifest does not list.
function Test-PackManifest {
  [CmdletBinding()]
  param([Parameter(Mandatory = $true)] [string]$Root)
  $rootFull = (Resolve-Path -LiteralPath $Root).Path.TrimEnd('\')
  $mf = Join-Path $rootFull 'PACK-MANIFEST.json'
  if (-not (Test-Path -LiteralPath $mf)) { return @('PACK-MANIFEST.json is missing') }
  $pm = Get-Content -LiteralPath $mf -Raw -Encoding UTF8 | ConvertFrom-Json
  $problems = New-Object System.Collections.Generic.List[string]
  $listed = @{}
  foreach ($e in @($pm.files)) {
    $key = ([string]$e.path).ToLowerInvariant()
    $listed[$key] = $true
    $p = Join-Path $rootFull (([string]$e.path) -replace '/', '\')
    if (-not (Test-Path -LiteralPath $p)) { $problems.Add("missing: $($e.path)"); continue }
    $h = (Get-FileHash -LiteralPath $p -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($h -ne [string]$e.sha256) { $problems.Add("changed: $($e.path)") }
  }
  foreach ($f in @(Get-ChildItem -LiteralPath $rootFull -Recurse -File -Force -ErrorAction SilentlyContinue)) {
    $rel = ($f.FullName.Substring($rootFull.Length + 1) -replace '\\', '/')
    if ($rel -eq 'PACK-MANIFEST.json' -or $rel -like 'work/*' -or $rel -like 'gate-results-*.zip') { continue }
    if (-not $listed.ContainsKey($rel.ToLowerInvariant())) { $problems.Add("not in the manifest: $rel") }
  }
  @($problems.ToArray())
}

# $true when -Text occurs in the file as ASCII or as UTF-16 (Rust and Windows strings are either).
function Test-PackExeStrings {
  [CmdletBinding()]
  param([Parameter(Mandatory = $true)] [string]$Path, [Parameter(Mandatory = $true)] [string]$Text)
  $b = [System.IO.File]::ReadAllBytes($Path)
  foreach ($enc in @([Text.Encoding]::ASCII, [Text.Encoding]::Unicode)) {
    $needle = $enc.GetBytes($Text)
    $limit = $b.Length - $needle.Length
    # IndexOf jumps to the next byte equal to the first one at native speed; only those are compared.
    $i = [Array]::IndexOf($b, $needle[0], 0)
    while ($i -ge 0 -and $i -le $limit) {
      $k = 1
      while ($k -lt $needle.Length -and $b[$i + $k] -eq $needle[$k]) { $k++ }
      if ($k -eq $needle.Length) { return $true }
      if ($i + 1 -ge $b.Length) { break }
      $i = [Array]::IndexOf($b, $needle[0], $i + 1)
    }
  }
  $false
}

# Pure. -Entries: objects with Path and Bytes. Returns lines for the console and the handoff note.
function Format-PackSizeReport {
  [CmdletBinding()]
  param([Parameter(Mandatory = $true)] $Entries, [double[]]$UsbGB = @(8, 16, 32, 64))
  $all = @($Entries)
  $total = [double](($all | Measure-Object -Property Bytes -Sum).Sum)
  $lines = New-Object System.Collections.Generic.List[string]
  $lines.Add(('pack size: {0:n2} GB in {1} files (1 GB = 10^9 bytes)' -f ($total / 1e9), $all.Count))
  $groups = $all | Group-Object { $p = ([string]$_.Path).Replace('\', '/'); $i = $p.IndexOf('/'); if ($i -lt 0) { '(root)' } else { $p.Substring(0, $i) } }
  foreach ($g in ($groups | Sort-Object { ($_.Group | Measure-Object -Property Bytes -Sum).Sum } -Descending)) {
    $s = [double](($g.Group | Measure-Object -Property Bytes -Sum).Sum)
    $lines.Add(('  {0,-12} {1,8:n2} GB  ({2} files)' -f $g.Name, ($s / 1e9), $g.Count))
  }
  $biggest = ($all | Sort-Object Bytes -Descending | Select-Object -First 1)
  if ($biggest) { $lines.Add(('largest file: {0} ({1:n2} GB){2}' -f $biggest.Path, ($biggest.Bytes / 1e9), $(if ($biggest.Bytes -ge 4GB) { ': does not fit FAT32' } else { ': every file is under 4 GB, so FAT32 also works' }))) }
  foreach ($u in $UsbGB) {
    $fits = $total -le ($u * 1e9 * 0.97)
    $lines.Add(('  a {0:n0} GB USB drive (about {1:n1} GB usable): {2}' -f $u, ($u * 0.93), $(if ($fits) { 'fits' } else { 'does not fit' })))
  }
  @($lines.ToArray())
}

Export-ModuleMember -Function New-PackWavSlice, Find-PackSpeechStart, Get-PackSecretPatternNames, Get-PackForbidden, Test-PackManifest, Test-PackExeStrings, Format-PackSizeReport
