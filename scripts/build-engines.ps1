<#
.SYNOPSIS
  Builds the wb-engine variants (avx2, avx512, vulkan by default; cuda on request) portably and checks
  that every flag and backend really reached the build.

.DESCRIPTION
  Every build sets GGML_NATIVE=OFF and the variant's CPU flags through environment variables, which
  whisper-rs-sys forwards to CMake (AGENTS.md non-negotiable 4). Each variant has its own target
  folder (target\engines\<variant>), so nothing is shared between them. After each build the script
  reads the CMake cache and the configure log back and FAILS if a flag or backend line is missing: a
  renamed or ignored option cannot silently drop a backend.

  Outputs: src-tauri\binaries\wb-engine-<variant>-x86_64-pc-windows-msvc.exe (git-ignored; the name
  Tauri's externalBin expects), target\engines\<variant>.result.json and, with -Evidence, the
  evidence text (docs\evidence\phase-2\engine-builds.txt): flags, backend lines, every DLL the engine
  needs beyond Windows with its size, and the size of each variant's folder.

  Takes D:\Projects\stelmar-build.lock for the whole run; refuses to start while a gate runs
  (stelmar-gate.lock, stelmar-gate-request.flag). The gate scripts refuse while this lock exists, so
  a build and a timing-sensitive run never overlap.
#>
[CmdletBinding()]
param(
  # cuda is NOT built by default (owner's decision, 11 Nov 2026: Vulkan only is shipped; see docs/GATES.md).
  # It stays buildable for development with -Variants cuda. The release bundle (src-tauri/tauri.engines.conf.json),
  # the gate pack and CI do not include it.
  [ValidateSet('avx2', 'avx512', 'vulkan', 'cuda')] [string[]]$Variants = @('avx2', 'avx512', 'vulkan'),
  # CUDA architectures (CMake syntax). 86-real is Ampere (RTX 30), 89 is Ada (RTX 40, the Legion) with
  # PTX for newer cards. Fewer architectures build faster and give a smaller engine.
  [string]$CudaArchitectures = '86-real;89',
  [string]$Evidence,
  # Where the per-variant target folders go. SHORT ON PURPOSE: the Vulkan backend builds its shader
  # generator as a nested CMake project, and under a long target path (the repository folder plus
  # release\build\whisper-rs-sys-<hash>\out\build\ggml\src\ggml-vulkan\vulkan-shaders-gen-prefix\...) the
  # compiler check inside it runs past Windows' 260-character limit and fails with "No CMAKE_C_COMPILER
  # could be found" (found 10 Nov 2026; docs/BUILD.md).
  [string]$TargetRoot = $(if (Test-Path 'D:\') { 'D:\wbt' } else { Join-Path $env:SystemDrive 'wbt' }),
  [switch]$Rebuild,
  # Write -Evidence from the saved results of earlier builds instead of building.
  [switch]$EvidenceOnly,
  [string]$BuildLock = 'D:\Projects\stelmar-build.lock',
  [string]$GateLock = 'D:\Projects\stelmar-gate.lock',
  [string]$GateRequest = 'D:\Projects\stelmar-gate-request.flag'
)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo = (Resolve-Path (Join-Path $here '..')).Path
Import-Module (Join-Path $here 'lib\PsEnv.psm1') -Force
Set-WbCleanPSModulePath
Set-WbInvariantCulture
Import-Module (Join-Path $here 'lib\GateRun.psm1') -Force
Import-Module (Join-Path $here 'lib\Redact.psm1') -Force

$triple = 'x86_64-pc-windows-msvc'
$binDir = Join-Path $repo 'src-tauri\binaries'

# What each variant must show in its CMake cache and configure log.
$cpuFlags = @{
  'GGML_NATIVE' = 'OFF'; 'GGML_AVX' = 'ON'; 'GGML_AVX2' = 'ON'; 'GGML_FMA' = 'ON'; 'GGML_F16C' = 'ON'
}
function Get-VariantPlan([string]$v) {
  $flags = [ordered]@{}
  foreach ($k in $cpuFlags.Keys) { $flags[$k] = $cpuFlags[$k] }
  $flags['GGML_AVX512'] = $(if ($v -eq 'avx512') { 'ON' } else { 'OFF' })
  $flags['GGML_CUDA'] = $(if ($v -eq 'cuda') { 'ON' } else { 'OFF' })
  $flags['GGML_VULKAN'] = $(if ($v -eq 'vulkan') { 'ON' } else { 'OFF' })
  $lines = @('Including CPU backend', $(if ($v -eq 'avx512') { 'Adding CPU backend variant ggml-cpu: /arch:AVX512' } else { 'Adding CPU backend variant ggml-cpu: /arch:AVX2' }))
  if ($v -eq 'vulkan') { $lines += 'Including Vulkan backend' }
  if ($v -eq 'cuda') { $lines += 'Including CUDA backend' }
  $features = switch ($v) { 'vulkan' { @('--features', 'vulkan') } 'cuda' { @('--features', 'cuda') } default { @() } }
  [pscustomobject]@{ Flags = $flags; Lines = $lines; Features = $features }
}

function Write-BuildEvidence($results) {
  $inv = [Globalization.CultureInfo]::InvariantCulture
  $pin = (Select-String -LiteralPath (Join-Path $repo 'third_party\README.md') -Pattern '[0-9a-f]{64}' | Select-Object -First 1).Matches[0].Value
  $lines = @(
    'Engine builds (Phase 2)',
    "date: $((Get-Date).ToString('yyyy-MM-dd HH:mm', $inv)) (IST on the Legion)",
    "target root: $TargetRoot; whisper.cpp tree hash pinned in third_party/README.md: $pin",
    'every build: GGML_NATIVE=OFF, CMAKE_{C,CXX}_FLAGS_RELEASE=/O2 /Ob2 /DNDEBUG (checked in the CMake cache); no build ran during a timing-sensitive run (build lock)',
    ''
  )
  foreach ($r in $results) {
    $hello = (Invoke-WbNative -FilePath $r.Exe -ArgumentList @('hello')).Output | Where-Object { $_ -match '^\{' } | Select-Object -First 1 | ConvertFrom-Json
    $tree = $hello.whisper_cpp.tree_sha256
    $lines += "== $($r.Variant) : $($r.Seconds) s, $(([math]::Round($r.Bytes / 1MB, 2)).ToString($inv)) MB, SHA-256 $($r.Sha256)"
    $lines += "   whisper.cpp printed by the engine: $($hello.whisper_cpp.version) commit $($hello.whisper_cpp.commit)"
    $lines += "   tree hash printed by the engine: $tree ($(if ($tree -eq $pin) { 'equals the pin' } else { 'DIFFERS FROM THE PIN' }))"
    if ($r.CudaArchitectures) { $lines += "   CUDA architectures: $($r.CudaArchitectures)" }
    $lines += '   CMake cache:'
    $lines += $r.Flags | ForEach-Object { "     $_" }
    $lines += '   configure log lines:'
    $lines += $r.BackendLines | ForEach-Object { "     $_" }
    $lines += ''
  }
  $lines += 'DLLs each engine needs beyond Windows, and the size of each engines-only folder: see engines-info.txt (scripts/gates/engines-only.ps1).'
  Write-WbEvidence -Path $Evidence -Lines $lines
  Write-Host "evidence: $Evidence"
}

if ($EvidenceOnly) {
  # Evidence from the results of earlier builds (<TargetRoot>\<variant>.result.json); builds nothing.
  $loaded = @($Variants | ForEach-Object { Get-Content -LiteralPath (Join-Path $TargetRoot "$_.result.json") -Raw | ConvertFrom-Json })
  if (-not $Evidence) { throw '-EvidenceOnly needs -Evidence <file>' }
  Write-BuildEvidence $loaded
  return
}

if (Test-Path -LiteralPath $GateLock) { throw "A gate is running ($GateLock exists). Builds never overlap a timing-sensitive run; wait for it." }
if (Test-Path -LiteralPath $GateRequest) { throw "A gate sequence is requested or running ($GateRequest exists). Wait for it." }
if (Test-Path -LiteralPath $BuildLock) { throw "Another build holds $BuildLock. Wait for it (or delete the file if none is running)." }
New-Item -ItemType Directory -Force -Path $binDir | Out-Null
# The lock's folder exists on the Legion; on a CI runner it may not.
New-Item -ItemType Directory -Force -Path (Split-Path -Parent $BuildLock) | Out-Null
Set-Content -LiteralPath $BuildLock -Value "build=build-engines $($Variants -join ',')`r`nstarted=$((Get-Date).ToString('o'))`r`npid=$PID`r`n" -Encoding ascii

# The build folder of whisper-rs-sys (the one whose out\build holds the CMake cache), newest first.
# Cargo's layout of intermediate files changed with the toolchain: before Rust 1.100 it was
# release\build\whisper-rs-sys-<hash>\out\build; CI found no such folder after the stable toolchain moved to
# 1.100 on 12 Nov 2026 (release\build\whisper-rs-sys\<hash>\out\build is the layout the new cargo is
# believed to use). So the cache is searched for instead of assumed: any release\build\...\out\build\CMakeCache.txt
# under a folder named for whisper-rs-sys. Returns the folder that holds out\ (and the build script's output file).
function Find-WhisperSysBuildDir([string]$Target) {
  $buildDir = Join-Path $Target 'release\build'
  if (-not (Test-Path -LiteralPath $buildDir)) { return $null }
  $hits = @(Get-ChildItem -LiteralPath $buildDir -Recurse -Filter 'CMakeCache.txt' -File -ErrorAction SilentlyContinue |
      Where-Object { $_.FullName -match '[\\/]whisper-rs-sys[^\\/]*[\\/](?:[^\\/]+[\\/])?out[\\/]build[\\/]CMakeCache\.txt$' })
  $dirs = @($hits | ForEach-Object { Get-Item -LiteralPath (Split-Path -Parent (Split-Path -Parent (Split-Path -Parent $_.FullName))) })
  @($dirs | Sort-Object LastWriteTime -Descending)[0]
}

# What the build script printed (CMake's configure lines are in it): the file `output` in the old layout, the
# file `run\stdout` of the same folder in the new one (checked with a probe crate on Rust 1.100.0 and 1.99.1:
# same content). Returns $null when neither exists.
function Get-WhisperSysConfigureLog([string]$Dir) {
  foreach ($rel in 'output', 'run\stdout') {
    $p = Join-Path $Dir $rel
    if (Test-Path -LiteralPath $p -PathType Leaf) { return $p }
  }
  $null
}

# What to look at when the CMake cache is not where the checks expect it (a CI run failed this way on
# 13 Nov 2026 and the build log was only on the runner): the end of the cargo log, and what every
# whisper-rs-sys build folder holds. Printed, never thrown: the caller still fails with its own message.
function Write-EngineBuildDiagnostics([string]$Target, [string]$Log) {
  try {
    Write-Host "---- diagnostics: no CMake cache found under $Target\release\build\whisper-rs-sys-*\out\build"
    if (Test-Path -LiteralPath $Log) {
      Write-Host "---- last 200 lines of the cargo build log ($Log)"
      Get-Content -LiteralPath $Log -Tail 200 | ForEach-Object { Write-Host "  $_" }
    } else {
      Write-Host "---- no cargo build log at $Log"
    }
    $buildDir = Join-Path $Target 'release\build'
    if (-not (Test-Path -LiteralPath $buildDir)) { Write-Host "---- $buildDir does not exist"; return }
    Write-Host ("---- everything directly under ${buildDir}: " + ((Get-ChildItem -LiteralPath $buildDir -Force -ErrorAction SilentlyContinue | Select-Object -First 80 | ForEach-Object { $_.Name }) -join ', '))
    $dirs = @(Get-ChildItem -LiteralPath $buildDir -Directory -Filter 'whisper-rs-sys*' -ErrorAction SilentlyContinue)
    Write-Host "---- whisper-rs-sys* folders under $buildDir : $($dirs.Count)"
    foreach ($d in $dirs) {
      Write-Host "  $($d.Name) (last write $($d.LastWriteTime.ToString('s')))"
      Write-Host ('    holds: ' + ((Get-ChildItem -LiteralPath $d.FullName -Force -ErrorAction SilentlyContinue | ForEach-Object { $_.Name }) -join ', '))
      $out = Join-Path $d.FullName 'out'
      Write-Host "    out\ exists: $(Test-Path -LiteralPath $out)"
      if (Test-Path -LiteralPath $out) {
        Write-Host ('    out\ holds: ' + ((Get-ChildItem -LiteralPath $out -Force -ErrorAction SilentlyContinue | ForEach-Object { $_.Name }) -join ', '))
        $ob = Join-Path $out 'build'
        Write-Host "    out\build\ exists: $(Test-Path -LiteralPath $ob)"
        if (Test-Path -LiteralPath $ob) {
          Write-Host ('    out\build\ holds: ' + ((Get-ChildItem -LiteralPath $ob -Force -ErrorAction SilentlyContinue | Select-Object -First 40 | ForEach-Object { $_.Name }) -join ', '))
        }
        $caches = @(Get-ChildItem -LiteralPath $out -Recurse -Filter 'CMakeCache.txt' -File -ErrorAction SilentlyContinue)
        Write-Host "    CMakeCache.txt anywhere under out\: $($caches.Count)"
        foreach ($c in $caches | Select-Object -First 10) { Write-Host "      $($c.FullName)" }
      }
      foreach ($n in 'output', 'stderr', 'run\stdout', 'run\stderr') {
        $f = Join-Path $d.FullName $n
        if (Test-Path -LiteralPath $f) {
          Write-Host "    last 30 lines of $n :"
          Get-Content -LiteralPath $f -Tail 30 -ErrorAction SilentlyContinue | ForEach-Object { Write-Host "      $_" }
        }
      }
    }
  } catch {
    Write-Host "---- diagnostics failed: $($_.Exception.Message)"
  }
}

function Get-CacheValue([string]$Cache, [string]$Name) {
  $m = Select-String -LiteralPath $Cache -Pattern ("^{0}:[A-Z]+=(.*)$" -f [regex]::Escape($Name)) | Select-Object -First 1
  if ($m) { $m.Matches[0].Groups[1].Value.Trim() } else { $null }
}

# MSVC's own release defaults.
$releaseFlags = '/O2 /Ob2 /DNDEBUG'

$results = @()
try {
  Push-Location $repo
  foreach ($v in $Variants) {
    $plan = Get-VariantPlan $v
    $target = Join-Path $TargetRoot $v
    $log = "$target.build.log"
    # A clean environment: nothing from an earlier variant or the shell leaks into this build.
    foreach ($e in @(Get-ChildItem Env: | Where-Object { $_.Name -like 'GGML_*' -or $_.Name -like 'WHISPER_*' -or $_.Name -eq 'CMAKE_CUDA_ARCHITECTURES' -or $_.Name -like 'CMAKE_*_FLAGS_RELEASE' -or $_.Name -eq 'WB_ENGINE_VARIANT' })) {
      Remove-Item -LiteralPath "Env:$($e.Name)"
    }
    foreach ($k in $plan.Flags.Keys) { Set-Item -LiteralPath "Env:$k" -Value $plan.Flags[$k] }
    # The cmake crate replaces CMake's own release flags with the C compiler's flags minus any /O option, so
    # without these two lines ggml is compiled WITHOUT optimisation (found 10 Nov 2026: 10 to 17 times slower
    # than whisper-cli). whisper-rs-sys forwards every CMAKE_* variable.
    $env:CMAKE_C_FLAGS_RELEASE = $releaseFlags
    $env:CMAKE_CXX_FLAGS_RELEASE = $releaseFlags
    $env:WB_ENGINE_VARIANT = $v
    if ($v -eq 'cuda') { $env:CMAKE_CUDA_ARCHITECTURES = $CudaArchitectures }
    if ($Rebuild -and (Test-Path -LiteralPath $target)) { Remove-Item -LiteralPath $target -Recurse -Force }
    $started = Get-Date
    Write-Host "== $v : building (flags: $(($plan.Flags.GetEnumerator() | ForEach-Object { "$($_.Key)=$($_.Value)" }) -join ' '))"
    $cargoArgs = @('build', '--release', '-p', 'wb-engine', '--target-dir', $target) + $plan.Features
    # cargo writes progress to stderr: collect everything as text (no native-stderr redirect).
    $res = Invoke-WbNative -FilePath 'cargo' -ArgumentList $cargoArgs
    $res.Output + $res.Error | Set-Content -LiteralPath $log -Encoding utf8
    if ($res.ExitCode -ne 0) { throw "cargo build failed for $v (exit $($res.ExitCode)); see $log" }
    $seconds = [math]::Round(((Get-Date) - $started).TotalSeconds)

    $exe = Join-Path $target 'release\wb-engine.exe'
    $sysOut = Find-WhisperSysBuildDir -Target $target
    if (-not $sysOut) {
      Write-EngineBuildDiagnostics -Target $target -Log $log
      throw "$v : no whisper-rs-sys build output with a CMake cache"
    }
    $cache = Join-Path $sysOut.FullName 'out\build\CMakeCache.txt'
    $configureLog = Get-WhisperSysConfigureLog -Dir $sysOut.FullName
    if (-not $configureLog) {
      Write-EngineBuildDiagnostics -Target $target -Log $log
      throw "$v : no build script output (output or run\stdout) in $($sysOut.FullName)"
    }
    $configure = Get-Content -LiteralPath $configureLog

    $problems = @()
    $flagLines = @()
    foreach ($k in $plan.Flags.Keys) {
      $got = Get-CacheValue $cache $k
      $flagLines += "$k=$got"
      if ($got -ne $plan.Flags[$k]) { $problems += "$k is '$got' in the CMake cache, expected '$($plan.Flags[$k])'" }
    }
    $backendLines = @()
    foreach ($want in $plan.Lines) {
      $hit = $configure | Where-Object { $_ -like "*$want*" } | Select-Object -First 1
      if ($hit) { $backendLines += $hit.Trim() } else { $problems += "the configure log has no line '$want'" }
    }
    foreach ($k in 'CMAKE_C_FLAGS_RELEASE', 'CMAKE_CXX_FLAGS_RELEASE') {
      $got = Get-CacheValue $cache $k
      $flagLines += "$k=$got"
      if ($got -notmatch '/O2' -or $got -notmatch '/DNDEBUG') { $problems += "$k is '$got': the build is not optimised (needs /O2 and /DNDEBUG)" }
    }
    if ($v -ne 'avx512' -and ($configure | Where-Object { $_ -match 'arch:AVX512' })) { $problems += 'an AVX-512 CPU variant was configured in a build that must not have one' }
    if ($v -ne 'avx512' -and ($configure | Where-Object { $_ -match 'Adding CPU backend variant' -and $_ -match 'AVX512' })) { $problems += 'AVX-512 appears in the CPU variant line' }

    $dest = Join-Path $binDir "wb-engine-$v-$triple.exe"
    Copy-Item -LiteralPath $exe -Destination $dest -Force
    $sha = (Get-FileHash -LiteralPath $dest -Algorithm SHA256).Hash.ToLowerInvariant()
    $r = [pscustomobject]@{
      Variant = $v; Seconds = $seconds; Exe = $dest; Sha256 = $sha; Bytes = (Get-Item -LiteralPath $dest).Length
      Flags = $flagLines; BackendLines = $backendLines; Problems = $problems; Log = $log; Cache = $cache
      CudaArchitectures = $(if ($v -eq 'cuda') { $CudaArchitectures } else { $null })
    }
    $results += $r
    $r | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath "$target.result.json" -Encoding utf8
    if ($problems.Count -gt 0) { throw "$v : the build did not get what was asked for: $($problems -join '; ')" }
    Write-Host ("   ok: {0} ({1:n1} MB, {2} s)" -f $dest, ($r.Bytes / 1MB), $seconds)
  }
} finally {
  Pop-Location
  foreach ($e in @(Get-ChildItem Env: | Where-Object { $_.Name -like 'GGML_*' -or $_.Name -eq 'CMAKE_CUDA_ARCHITECTURES' -or $_.Name -like 'CMAKE_*_FLAGS_RELEASE' -or $_.Name -eq 'WB_ENGINE_VARIANT' })) { Remove-Item -LiteralPath "Env:$($e.Name)" }
  Remove-Item -LiteralPath $BuildLock -Force -ErrorAction SilentlyContinue
}
if ($Evidence) { Write-BuildEvidence $results }
Write-Host "done: $(($results | ForEach-Object { $_.Variant }) -join ', ')"
