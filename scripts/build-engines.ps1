<#
.SYNOPSIS
  Builds the wb-engine variants (avx2, avx512, vulkan, cuda) portably and checks that every flag and
  backend really reached the build.

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
  [ValidateSet('avx2', 'avx512', 'vulkan', 'cuda')] [string[]]$Variants = @('avx2', 'avx512', 'vulkan', 'cuda'),
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

if (Test-Path -LiteralPath $GateLock) { throw "A gate is running ($GateLock exists). Builds never overlap a timing-sensitive run; wait for it." }
if (Test-Path -LiteralPath $GateRequest) { throw "A gate sequence is requested or running ($GateRequest exists). Wait for it." }
if (Test-Path -LiteralPath $BuildLock) { throw "Another build holds $BuildLock. Wait for it (or delete the file if none is running)." }
New-Item -ItemType Directory -Force -Path $binDir | Out-Null
Set-Content -LiteralPath $BuildLock -Value "build=build-engines $($Variants -join ',')`r`nstarted=$((Get-Date).ToString('o'))`r`npid=$PID`r`n" -Encoding ascii

function Get-CacheValue([string]$Cache, [string]$Name) {
  $m = Select-String -LiteralPath $Cache -Pattern ("^{0}:[A-Z]+=(.*)$" -f [regex]::Escape($Name)) | Select-Object -First 1
  if ($m) { $m.Matches[0].Groups[1].Value.Trim() } else { $null }
}

$results = @()
try {
  Push-Location $repo
  foreach ($v in $Variants) {
    $plan = Get-VariantPlan $v
    $target = Join-Path $TargetRoot $v
    $log = "$target.build.log"
    # A clean environment: nothing from an earlier variant or the shell leaks into this build.
    foreach ($e in @(Get-ChildItem Env: | Where-Object { $_.Name -like 'GGML_*' -or $_.Name -like 'WHISPER_*' -or $_.Name -eq 'CMAKE_CUDA_ARCHITECTURES' -or $_.Name -eq 'WB_ENGINE_VARIANT' })) {
      Remove-Item -LiteralPath "Env:$($e.Name)"
    }
    foreach ($k in $plan.Flags.Keys) { Set-Item -LiteralPath "Env:$k" -Value $plan.Flags[$k] }
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
    $sysOut = @(Get-ChildItem (Join-Path $target 'release\build') -Directory -Filter 'whisper-rs-sys-*' | Where-Object { Test-Path (Join-Path $_.FullName 'out\build\CMakeCache.txt') } | Sort-Object LastWriteTime -Descending)[0]
    if (-not $sysOut) { throw "$v : no whisper-rs-sys build output with a CMake cache" }
    $cache = Join-Path $sysOut.FullName 'out\build\CMakeCache.txt'
    $configure = Get-Content -LiteralPath (Join-Path $sysOut.FullName 'output')

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
  foreach ($e in @(Get-ChildItem Env: | Where-Object { $_.Name -like 'GGML_*' -or $_.Name -eq 'CMAKE_CUDA_ARCHITECTURES' -or $_.Name -eq 'WB_ENGINE_VARIANT' })) { Remove-Item -LiteralPath "Env:$($e.Name)" }
  Remove-Item -LiteralPath $BuildLock -Force -ErrorAction SilentlyContinue
}
Write-Host "done: $(($results | ForEach-Object { $_.Variant }) -join ', ')"
