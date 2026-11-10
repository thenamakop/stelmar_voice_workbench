<#
.SYNOPSIS
  Checks that every CMake option the engine builds depend on still exists, under the same name, in
  the vendored whisper.cpp, so a renamed flag cannot silently drop a backend or a CPU setting.

.DESCRIPTION
  Names come from two places:
    * the .define("NAME", ...) calls in the vendored whisper-rs-sys build.rs, which are the options
      whisper-rs-sys sets itself (GGML_CUDA, GGML_VULKAN, WHISPER_BUILD_EXAMPLES, ...), and
    * the options scripts/build-engines.ps1 sets through the environment (GGML_NATIVE, GGML_AVX2, ...),
      which whisper-rs-sys forwards to CMake without looking at them: a name that CMake does not know
      is ignored without a word.
  A name counts as present when a CMakeLists.txt or *.cmake file of the vendored tree defines it with
  option(), cmake_dependent_option() or set(). Options that belong to a backend this project does not
  build (Metal, HIP, SYCL, OpenBLAS, CoreML) are checked and reported but do not fail the check.
#>
[CmdletBinding()]
param(
  [string]$Tree,
  [string]$SysBuildRs
)
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path (Split-Path -Parent $MyInvocation.MyCommand.Path) '..')).Path
if (-not $Tree) { $Tree = Join-Path $repo 'third_party\whisper.cpp' }
if (-not $SysBuildRs) { $SysBuildRs = Join-Path $repo 'third_party\whisper-rs-sys\build.rs' }

# Options build-engines.ps1 sets in the environment (see docs/BUILD.md).
$engineOptions = @('GGML_NATIVE', 'GGML_AVX', 'GGML_AVX2', 'GGML_FMA', 'GGML_F16C', 'GGML_AVX512', 'GGML_CUDA', 'GGML_VULKAN')
# CMake's own variables.
$builtin = @('BUILD_SHARED_LIBS')
# Options of backends we do not build.
$optional = @('WHISPER_COREML', 'WHISPER_COREML_ALLOW_FALLBACK', 'GGML_HIP', 'GGML_METAL', 'GGML_METAL_NDEBUG', 'GGML_METAL_EMBED_LIBRARY', 'GGML_BLAS', 'GGML_BLAS_VENDOR', 'GGML_SYCL', 'GGML_OPENMP')

$fromBuildRs = @(Select-String -LiteralPath $SysBuildRs -Pattern '\.define\("([A-Za-z0-9_]+)"' -AllMatches | ForEach-Object { $_.Matches } | ForEach-Object { $_.Groups[1].Value })
$names = @($fromBuildRs + $engineOptions | Sort-Object -Unique)

$cmakeFiles = @(Get-ChildItem -LiteralPath $Tree -Recurse -File | Where-Object { $_.Name -eq 'CMakeLists.txt' -or $_.Extension -eq '.cmake' })
$text = ($cmakeFiles | ForEach-Object { Get-Content -LiteralPath $_.FullName -Raw }) -join "`n"

$missing = @()
Write-Host ("whisper-rs-sys sets {0} option(s) itself; build-engines.ps1 sets {1}; checking {2} name(s) against {3} CMake file(s) of the vendored tree" -f $fromBuildRs.Count, $engineOptions.Count, $names.Count, $cmakeFiles.Count)
foreach ($n in $names) {
  if ($builtin -contains $n) { Write-Host ("  {0,-34} CMake builtin" -f $n); continue }
  $esc = [regex]::Escape($n)
  $def = [regex]::Match($text, "(?im)^\s*(option|cmake_dependent_option|set)\s*\(\s*$esc\b")
  if ($def.Success) {
    Write-Host ("  {0,-34} defined ({1})" -f $n, $def.Groups[1].Value.ToLowerInvariant())
  } elseif ($optional -contains $n) {
    Write-Host ("  {0,-34} NOT FOUND (optional: a backend this project does not build)" -f $n)
  } else {
    Write-Host ("  {0,-34} NOT FOUND" -f $n)
    $missing += $n
  }
}
if ($missing.Count -gt 0) {
  Write-Host ("FAIL: not defined by the vendored whisper.cpp: {0}. A renamed option would be ignored without an error." -f ($missing -join ', '))
  exit 1
}
Write-Host 'OK: every option the build depends on is defined by the vendored whisper.cpp'
