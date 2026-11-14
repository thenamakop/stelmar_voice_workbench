<#
.SYNOPSIS
  Assembles gate-pack\: the folder one person copies to a 16 GB laptop to run every pending gate.

.DESCRIPTION
  Copies the gate scripts and their modules, the built programs (gate-build app, release app,
  wb-testbench, the engines), the models the speed tables use and the public test clips into
  -Out (default <repo>\gate-pack, git-ignored), cuts the short English clip the network-off gate plays,
  writes PACK-MANIFEST.json (SHA-256 of every file, the source commit, the build time) and README.txt,
  and prints the size against USB drives. Nothing here builds a program; build first
  (scripts\gates\build-gate.ps1, cargo build --release -p wb-testbench, scripts\build-engines.ps1).

  The pack holds no source code, no development tool, no wb-emulate, no private audio and no secret;
  scripts\check-gate-pack.ps1 proves it. A gate whose script is not in scripts\gate-pack\gates is
  simply not in the pack: the runner and the summary say "not in this pack".

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File .\scripts\build-gate-pack.ps1
  powershell -ExecutionPolicy Bypass -File .\scripts\build-gate-pack.ps1 -ArtifactsRoot D:\Projects\stelmar_voice_workbench -Force
#>
[CmdletBinding()]
param(
  [string]$Out = '',
  # Where the built programs, the engines and the model files are (they are git-ignored, so a worktree
  # does not have them). Default: this repository.
  [string]$ArtifactsRoot = '',
  # Folder that holds the public FLEURS clips the batch gate uses (data\fleurs-*\...), when present.
  [string]$DataSource = '',
  [string[]]$Variants = @('avx2', 'avx512', 'vulkan'),
  # Allow a build from a working tree with uncommitted changes in scripts\ (the commit is recorded with
  # a "-dirty" mark).
  [switch]$AllowDirty,
  [switch]$Force,
  [switch]$SkipModels,
  # Extra programs (for example the supervisor kill-test executable): path = relative path in the pack.
  [hashtable]$Extra = @{},
  [double[]]$UsbGB = @(8, 16, 32, 64)
)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo = (Resolve-Path (Join-Path $here '..')).Path
if (-not $Out) { $Out = Join-Path $repo 'gate-pack' }
if (-not $ArtifactsRoot) { $ArtifactsRoot = $repo }
Import-Module (Join-Path $repo 'scripts\lib\PackBuild.psm1') -Force
Import-Module (Join-Path $repo 'scripts\lib\PackCore.psm1') -Force

function Fail([string]$m) { Write-Host "build-gate-pack: $m" -ForegroundColor Red; exit 1 }
function Note([string]$m) { Write-Host "build-gate-pack: $m" }

# ---- The source commit ----
$commit = (& git -C $repo rev-parse HEAD 2>$null)
if (-not $commit) { Fail 'cannot read the git commit' }
$dirty = @(& git -C $repo status --porcelain -- scripts crates/wb-testbench docs/GATES.md 2>$null | Where-Object { $_ -and $_ -notmatch '^\?\? ' })
if ($dirty.Count -gt 0 -and -not $AllowDirty) { Fail ("uncommitted changes in scripts\ or crates\wb-testbench (commit them or pass -AllowDirty):`n  " + ($dirty -join "`n  ")) }
$commitText = $(if ($dirty.Count -gt 0) { "$commit-dirty" } else { [string]$commit })

# ---- The registry must be valid and match docs\GATES.md ----
$manifestPath = Join-Path $repo 'scripts\gate-pack\gates.manifest.json'
$manifest = Read-PackManifest -Path $manifestPath
$bad = @(Test-PackLedgerMatches -Manifest $manifest -GatesMdPath (Join-Path $repo 'docs\GATES.md') | Where-Object { $_.Count -ne 1 })
if ($bad.Count -gt 0) { Fail ("ledger entries that do not match exactly one row of docs\GATES.md:`n  " + (($bad | ForEach-Object { "$($_.Gate): '$($_.Match)' matches $($_.Count)" }) -join "`n  ")) }

# ---- The output folder ----
if (Test-Path -LiteralPath $Out) {
  if (-not $Force) { Fail "$Out exists; pass -Force to replace it" }
  Remove-Item -LiteralPath $Out -Recurse -Force
}
New-Item -ItemType Directory -Force -Path $Out | Out-Null
$Out = (Resolve-Path $Out).Path

function Copy-Into([string]$From, [string]$ToRel) {
  if (-not (Test-Path -LiteralPath $From)) { Fail "missing $From" }
  $to = Join-Path $Out $ToRel
  New-Item -ItemType Directory -Force -Path (Split-Path -Parent $to) | Out-Null
  Copy-Item -LiteralPath $From -Destination $to -Force
}

# ---- Scripts ----
Copy-Into (Join-Path $repo 'scripts\gate-pack\run-gates.ps1') 'run-gates.ps1'
Copy-Into $manifestPath 'scripts\gate-pack\gates.manifest.json'
foreach ($f in @(Get-ChildItem -LiteralPath (Join-Path $repo 'scripts\gate-pack\gates') -Filter '*.ps1')) { Copy-Into $f.FullName ('scripts\gate-pack\gates\' + $f.Name) }
# Every library module, except the ones that exist only on the development side.
foreach ($f in @(Get-ChildItem -LiteralPath (Join-Path $repo 'scripts\lib') -File | Where-Object { $_.Extension -in '.psm1', '.ps1' -and $_.Name -notin @('PackBuild.psm1') })) { Copy-Into $f.FullName ('scripts\lib\' + $f.Name) }
# The older gate scripts the pack calls. None of them needs Python, Cargo or Git on the laptop.
$legacy = @('recorder-gate.ps1', 'recorder-gate-inner.ps1', 'kill-gate.ps1', 'network-off.ps1', 'engines-only.ps1', 'batch-scripts.ps1', 'rename-engines.ps1', 'xrun-correlate.ps1', 'rig-selftest.ps1')
foreach ($n in $legacy) {
  $p = Join-Path $repo ('scripts\gates\' + $n)
  if (Test-Path -LiteralPath $p) { Copy-Into $p ('scripts\gates\' + $n) } else { Note "note: scripts\gates\$n does not exist; gates that call it will say so" }
}
Copy-Into (Join-Path $repo 'scripts\offline-check.ps1') 'scripts\offline-check.ps1'
if (Test-Path -LiteralPath (Join-Path $repo 'scripts\gate-pack\README.txt')) { Copy-Into (Join-Path $repo 'scripts\gate-pack\README.txt') 'README.txt' } else { Fail 'scripts\gate-pack\README.txt is missing' }

# The one-click starter: Windows PowerShell with the execution policy of this one run set to Bypass (the
# machine's policy is not changed), in the pack's folder.
$cmd = "@echo off`r`ncd /d `"%~dp0`"`r`npowershell.exe -NoProfile -ExecutionPolicy Bypass -File `"%~dp0run-gates.ps1`" %*`r`npause`r`n"
[IO.File]::WriteAllText((Join-Path $Out 'Start-Gates.cmd'), $cmd, [Text.Encoding]::ASCII)

# ---- Programs ----
$art = (Resolve-Path $ArtifactsRoot).Path
Copy-Into (Join-Path $art 'target\gate\stelmar-voice-workbench.exe') 'target\gate\stelmar-voice-workbench.exe'
Copy-Into (Join-Path $art 'target\release\stelmar-voice-workbench.exe') 'target\release\stelmar-voice-workbench.exe'
Copy-Into (Join-Path $art 'target\release\wb-testbench.exe') 'target\release\wb-testbench.exe'
foreach ($v in $Variants) {
  $n = "wb-engine-$v-x86_64-pc-windows-msvc.exe"
  Copy-Into (Join-Path $art ('src-tauri\binaries\' + $n)) ('src-tauri\binaries\' + $n)
}
foreach ($k in $Extra.Keys) { Copy-Into ([string]$Extra[$k]) ([string]$k) }

# ---- Models (each is a public model; the list is the speed table's) ----
$models = @('ggml-small.bin', 'ggml-medium.bin', 'ggml-large-v3-turbo-q5_0.bin', 'ggml-large-v3-q5_0.bin', 'converted\ggml-hinglish-q5_0.bin', 'ggml-silero-v6.2.0.bin')
Copy-Into (Join-Path $repo 'models\manifest.json') 'models\manifest.json'
if (-not $SkipModels) { foreach ($m in $models) { Copy-Into (Join-Path $art ('models\' + $m)) ('models\' + $m) } }

# ---- Clips (public FLEURS audio, CC BY 4.0) ----
$allowedWav = New-Object System.Collections.Generic.List[string]
foreach ($f in @(Get-ChildItem -LiteralPath (Join-Path $repo 'resources\bench') -File)) {
  Copy-Into $f.FullName ('resources\bench\' + $f.Name)
  if ($f.Extension -eq '.wav') { $allowedWav.Add('resources/bench/' + $f.Name.ToLowerInvariant()) }
}
New-Item -ItemType Directory -Force -Path (Join-Path $Out 'data') | Out-Null
$slice = New-PackWavSlice -Source (Join-Path $repo 'resources\bench\bench-en.wav') -Dest (Join-Path $Out 'data\net-clip-en.wav') -StartSeconds 6 -Seconds 6
$allowedWav.Add('data/net-clip-en.wav')
([ordered]@{ source = 'resources/bench/bench-en.wav (public FLEURS English, CC BY 4.0)'; start_s = 6; duration_s = $slice.duration_s; rate = $slice.rate } | ConvertTo-Json) | Set-Content -LiteralPath (Join-Path $Out 'data\net-clip-en.json') -Encoding UTF8
if ($DataSource -and (Test-Path -LiteralPath $DataSource)) {
  foreach ($f in @(Get-ChildItem -LiteralPath $DataSource -Recurse -File)) {
    $rel = $f.FullName.Substring((Resolve-Path $DataSource).Path.TrimEnd('\').Length + 1)
    Copy-Into $f.FullName ('data\' + $rel)
    if ($f.Extension -eq '.wav') { $allowedWav.Add(('data/' + $rel).Replace('\', '/').ToLowerInvariant()) }
  }
}

# ---- The manifest ----
$entries = New-Object System.Collections.Generic.List[object]
foreach ($f in @(Get-ChildItem -LiteralPath $Out -Recurse -File)) {
  $rel = ($f.FullName.Substring($Out.Length + 1) -replace '\\', '/')
  $entries.Add([ordered]@{ path = $rel; bytes = $f.Length; sha256 = (Get-FileHash -LiteralPath $f.FullName -Algorithm SHA256).Hash.ToLowerInvariant() })
}
$gatesInPack = @(@($manifest.gates) | Where-Object { Test-Path -LiteralPath (Join-Path $Out ('scripts\gate-pack\' + ([string]$_.script -replace '/', '\'))) } | ForEach-Object { [string]$_.id })
$gatesMissing = @(@($manifest.gates) | Where-Object { $gatesInPack -notcontains [string]$_.id } | ForEach-Object { [string]$_.id })
$pm = [ordered]@{
  pack_version = 1
  source_commit = $commitText
  built_at = (Get-Date).ToString('o')
  gates_in_pack = $gatesInPack
  gates_not_in_pack = $gatesMissing
  allowed_wav = @($allowedWav.ToArray())
  files = @($entries.ToArray())
}
($pm | ConvertTo-Json -Depth 6) | Set-Content -LiteralPath (Join-Path $Out 'PACK-MANIFEST.json') -Encoding UTF8

# ---- Report ----
Note "built $Out from commit $commitText"
Note ("gates in the pack ({0}): {1}" -f $gatesInPack.Count, ($gatesInPack -join ', '))
if ($gatesMissing.Count -gt 0) { Note ("NOT in this pack ({0}): {1}" -f $gatesMissing.Count, ($gatesMissing -join ', ')) }
$report = @(Format-PackSizeReport -Entries @($entries.ToArray() | ForEach-Object { [pscustomobject]@{ Path = $_.path; Bytes = [int64]$_.bytes } }) -UsbGB $UsbGB)
foreach ($l in $report) { Write-Host $l }
New-Item -ItemType Directory -Force -Path (Join-Path $repo 'target') | Out-Null
Set-Content -LiteralPath (Join-Path $repo 'target\gate-pack-size.txt') -Value $report -Encoding UTF8
Note 'next: powershell -File scripts\check-gate-pack.ps1'
