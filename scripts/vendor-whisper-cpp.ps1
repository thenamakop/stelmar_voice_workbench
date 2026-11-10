<#
.SYNOPSIS
  Copies the whisper.cpp source the engines build from into third_party/whisper.cpp, pinned to one
  release tag and one commit. Development time only (it downloads); the app and the builds never
  fetch anything.

.DESCRIPTION
  1. shallow-clones the tag into a scratch folder,
  2. refuses any commit other than -ExpectedCommit,
  3. copies only what the build needs (the same list whisper-rs-sys 0.15 ships): CMakeLists.txt,
     LICENSE, cmake/, include/whisper.h, src/, ggml/CMakeLists.txt, ggml/cmake/, ggml/include/*.h,
     ggml/src/, bindings/CMakeLists.txt and bindings/javascript/package-tmpl.json. No models,
     samples, examples, tests, scripts or binaries,
  4. prints the tree hash (scripts/check-vendor.mjs) to put in third_party/README.md.

  The tree is committed as a copy, not as a submodule, so builds work offline after the first clone.
#>
[CmdletBinding()]
param(
  [string]$Tag = 'v1.9.5',
  [string]$ExpectedCommit = 'd1be6fde11ac6e0407606b4e42fe72d34add8037',
  [string]$Url = 'https://github.com/ggml-org/whisper.cpp',
  [string]$Work
)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo = (Resolve-Path (Join-Path $here '..')).Path
Import-Module (Join-Path $repo 'scripts\lib\PsEnv.psm1') -Force
Set-WbCleanPSModulePath
Import-Module (Join-Path $repo 'scripts\lib\GateRun.psm1') -Force

if (-not $Work) { $Work = Join-Path ([System.IO.Path]::GetTempPath()) "wb-vendor-whispercpp-$([guid]::NewGuid().ToString('N').Substring(0, 8))" }
$clone = Join-Path $Work 'whisper.cpp'
$dest = Join-Path $repo 'third_party\whisper.cpp'

New-Item -ItemType Directory -Force -Path $Work | Out-Null
try {
  Write-Host "cloning $Url at $Tag into $clone (shallow) ..."
  $res = Invoke-WbNative -FilePath 'git' -ArgumentList @('clone', '--depth', '1', '--branch', $Tag, $Url, $clone)
  if ($res.ExitCode -ne 0) { throw "git clone failed: $((@($res.Error) + @($res.Output)) -join ' ')" }
  $head = (Invoke-WbNative -FilePath 'git' -ArgumentList @('-C', $clone, 'rev-parse', 'HEAD')).Output | Select-Object -First 1
  if ($head -ne $ExpectedCommit) { throw "Tag $Tag is commit $head, not the pinned $ExpectedCommit. Not copying." }
  Write-Host "commit $head matches the pin"

  if (Test-Path -LiteralPath $dest) { Remove-Item -LiteralPath $dest -Recurse -Force }
  New-Item -ItemType Directory -Force -Path $dest | Out-Null
  $items = @(
    @{ From = 'CMakeLists.txt'; File = $true },
    @{ From = 'LICENSE'; File = $true },
    @{ From = 'cmake'; File = $false },
    @{ From = 'include\whisper.h'; File = $true },
    @{ From = 'src'; File = $false },
    @{ From = 'ggml\CMakeLists.txt'; File = $true },
    @{ From = 'ggml\cmake'; File = $false },
    @{ From = 'ggml\src'; File = $false },
    @{ From = 'bindings\CMakeLists.txt'; File = $true },
    @{ From = 'bindings\javascript\package-tmpl.json'; File = $true }
  )
  foreach ($it in $items) {
    $src = Join-Path $clone $it.From
    if (-not (Test-Path -LiteralPath $src)) { Write-Host "note: $($it.From) does not exist in $Tag; skipped"; continue }
    $to = Join-Path $dest $it.From
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $to) | Out-Null
    Copy-Item -LiteralPath $src -Destination $to -Recurse -Force
  }
  # ggml/include/*.h only (headers, no subfolders).
  $gi = Join-Path $dest 'ggml\include'
  New-Item -ItemType Directory -Force -Path $gi | Out-Null
  Get-ChildItem -LiteralPath (Join-Path $clone 'ggml\include') -Filter '*.h' -File | Copy-Item -Destination $gi -Force

  $count = @(Get-ChildItem -LiteralPath $dest -Recurse -File).Count
  $bytes = (Get-ChildItem -LiteralPath $dest -Recurse -File | Measure-Object -Property Length -Sum).Sum
  Write-Host ("vendored {0} files, {1:n1} MB into {2}" -f $count, ($bytes / 1MB), $dest)
  Push-Location $repo
  try { node scripts/check-vendor.mjs --print } finally { Pop-Location }
  Write-Host 'Put the tree-sha256 and files lines into third_party/README.md, then run: node scripts/check-vendor.mjs'
} finally {
  Remove-Item -LiteralPath $Work -Recurse -Force -ErrorAction SilentlyContinue
}
