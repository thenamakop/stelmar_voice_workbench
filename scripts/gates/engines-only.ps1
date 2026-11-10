<#
.SYNOPSIS
  Phase 2 engine gates 1 to 3: `info` from an engines-only folder with no development tools on PATH.

.DESCRIPTION
  Copies each built engine (and the DLLs it needs beyond Windows) into a folder of its own under
  -Root, then runs `wb-engine info` there with PATH reduced to the Windows folders, once under each
  profile (emu-low, emu-mid, native) through wb-emulate. Evidence per variant and profile: exit code,
  the whisper.cpp version, commit and tree hash printed at runtime, the backends, and each device
  once (kind, memory, shared memory, driver version) with the number of duplicate entries dropped.

  Expected: avx2 exits 0; avx512 exits 3 with a clear message on a CPU without AVX-512; vulkan lists
  the Intel UHD once (and, natively, the RTX 4070 too); cuda lists the RTX 4070 (it needs the CUDA
  runtime DLLs, which are copied beside it for this test only; whether they may be redistributed is a
  Phase 8 question).

  This is not timing-sensitive; it takes no gate lock. It runs on the laptops without any development
  tool: copy the folder made by -Root and run this script there with -Emulate omitted (profile native).
#>
[CmdletBinding()]
param(
  [string[]]$Variants = @('avx2', 'avx512', 'vulkan', 'cuda'),
  [string[]]$Profiles = @('emu-low', 'emu-mid', 'native'),
  [string]$Root = 'D:\STELMAR-Gate\engines-only',
  [string]$Out,
  [string]$Emulate
)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo = (Resolve-Path (Join-Path $here '..\..')).Path
if (-not $Emulate) { $Emulate = Join-Path $repo 'target\release\wb-emulate.exe' }
if (-not $Out) { $Out = Join-Path $repo 'docs\evidence\phase-2\engines-info.txt' }
Import-Module (Join-Path $repo 'scripts\lib\PsEnv.psm1') -Force
Set-WbCleanPSModulePath
Set-WbInvariantCulture
Import-Module (Join-Path $repo 'scripts\lib\GateRun.psm1') -Force
Import-Module (Join-Path $repo 'scripts\lib\Redact.psm1') -Force

$triple = 'x86_64-pc-windows-msvc'
$sys32 = Join-Path $env:SystemRoot 'System32'
$minPath = "$sys32;$env:SystemRoot"

function Get-DumpbinPath {
  $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
  if (-not (Test-Path $vswhere)) { return $null }
  $vs = (& $vswhere -latest -products * -property installationPath) | Select-Object -First 1
  if (-not $vs) { return $null }
  Get-ChildItem (Join-Path $vs 'VC\Tools\MSVC') -Recurse -Filter dumpbin.exe -ErrorAction SilentlyContinue | Where-Object { $_.FullName -match 'Hostx64\\x64' } | Select-Object -First 1 -ExpandProperty FullName
}

# DLLs the engine needs beyond Windows: names from dumpbin, minus anything in System32. Found in the
# CUDA toolkit's bin folders. Recursive (a needed DLL's own needs are followed).
function Get-ExtraDlls([string]$Exe, [string]$Dumpbin) {
  $seen = @{}
  $queue = New-Object System.Collections.Queue
  $queue.Enqueue($Exe)
  $cudaBins = @()
  if ($env:CUDA_PATH) { $cudaBins = @((Join-Path $env:CUDA_PATH 'bin'), (Join-Path $env:CUDA_PATH 'bin\x64')) }
  $extra = @()
  while ($queue.Count -gt 0) {
    $f = $queue.Dequeue()
    $res = Invoke-WbNative -FilePath $Dumpbin -ArgumentList @('/dependents', $f)
    foreach ($l in $res.Output) {
      if ($l -match '^\s+(\S+\.dll)\s*$') {
        $name = $Matches[1]
        if ($seen.ContainsKey($name.ToLowerInvariant())) { continue }
        $seen[$name.ToLowerInvariant()] = $true
        if (Test-Path (Join-Path $sys32 $name)) { continue }
        if ($name -like 'api-ms-*' -or $name -like 'ext-ms-*') { continue }
        $found = $cudaBins | ForEach-Object { Join-Path $_ $name } | Where-Object { Test-Path $_ } | Select-Object -First 1
        if ($found) { $extra += $found; $queue.Enqueue($found) } else { $extra += "MISSING:$name" }
      }
    }
  }
  $extra
}

$dumpbin = Get-DumpbinPath
$lines = @(
  'Engines-only info check (Phase 2)',
  "date: $((Get-Date).ToString('yyyy-MM-dd HH:mm', [Globalization.CultureInfo]::InvariantCulture)) (IST on the Legion)",
  "PATH while running: $minPath (no cargo, cmake, CUDA, Vulkan SDK or Python)",
  'overlay tools closed: ' + $(if ((Get-WbOverlayTools).Count -eq 0) { 'yes' } else { 'NO' }),
  ''
)
$summary = @()
foreach ($v in $Variants) {
  $src = Join-Path $repo "src-tauri\binaries\wb-engine-$v-$triple.exe"
  if (-not (Test-Path $src)) { $lines += "$v : engine not built ($src missing)"; $summary += "$v : not built"; continue }
  $dir = Join-Path $Root $v
  if (Test-Path $dir) { Remove-Item $dir -Recurse -Force }
  New-Item -ItemType Directory -Force -Path $dir | Out-Null
  $exe = Join-Path $dir 'wb-engine.exe'
  Copy-Item $src $exe
  $dlls = @()
  if ($dumpbin) { $dlls = @(Get-ExtraDlls $src $dumpbin) }
  foreach ($d in $dlls) { if ($d -notlike 'MISSING:*') { Copy-Item $d $dir } }
  $size = (Get-ChildItem $dir -File | Measure-Object Length -Sum).Sum
  $lines += "== $v : engines-only folder $(([math]::Round($size / 1MB, 1)).ToString([Globalization.CultureInfo]::InvariantCulture)) MB; extra DLLs: $(if ($dlls.Count) { ($dlls | ForEach-Object { Split-Path $_ -Leaf }) -join ', ' } else { 'none' })"
  foreach ($p in $Profiles) {
    $saved = $env:PATH
    $env:PATH = $minPath
    try {
      $argsList = if ($p -eq 'native') { @('info') } else { @('--profile', $p, '--', (ConvertTo-WbArg $exe), 'info') }
      $res = Invoke-WbNative -FilePath $(if ($p -eq 'native') { $exe } else { $Emulate }) -ArgumentList $argsList
    } finally { $env:PATH = $saved }
    $events = @($res.Output | Where-Object { $_ -match '^\{' } | ForEach-Object { try { $_ | ConvertFrom-Json } catch { $null } } | Where-Object { $_ })
    $hello = $events | Where-Object { $_.event -eq 'hello' } | Select-Object -First 1
    $info = $events | Where-Object { $_.event -eq 'info' } | Select-Object -First 1
    $err = $events | Where-Object { $_.event -eq 'error' } | Select-Object -First 1
    $lines += "-- profile $p : exit code $($res.ExitCode)"
    if ($hello) { $lines += "   whisper.cpp printed at runtime: $($hello.whisper_cpp.version) commit $($hello.whisper_cpp.commit) tree $($hello.whisper_cpp.tree_sha256)" }
    if ($err) { $lines += "   error event: $($err.code): $($err.message)" }
    foreach ($e in $res.Error) { if ("$e".Trim()) { $lines += "   stderr: $("$e".Trim())" } }
    if ($info) {
      $lines += "   backends: $($info.backends -join ', '); logical processors allowed $($info.logical_processors_allowed); available RAM $([math]::Round($info.available_ram_bytes / 1GB, 1).ToString([Globalization.CultureInfo]::InvariantCulture)) GiB; raw device entries $($info.raw_device_count)"
      foreach ($d in $info.devices) {
        $mem = [math]::Round($d.total_memory_bytes / 1GB, 1).ToString([Globalization.CultureInfo]::InvariantCulture)
        $shared = if ($d.shared_memory_bytes) { ", shared with RAM up to $([math]::Round($d.shared_memory_bytes / 1GB, 1).ToString([Globalization.CultureInfo]::InvariantCulture)) GiB" } else { '' }
        $lines += "   device $($d.index): $($d.backend) / $($d.description) / $($d.kind) / $mem GiB$shared / driver $($d.driver_version) / duplicates dropped $($d.duplicates_dropped)"
      }
      foreach ($n in $info.notes) { $lines += "   note: $n" }
    }
    $summary += "$v/$p exit $($res.ExitCode)"
  }
  $lines += ''
}
$lines += 'summary: ' + ($summary -join '; ')
New-Item -ItemType Directory -Force -Path (Split-Path $Out) | Out-Null
Write-WbEvidence -Path $Out -Lines $lines
Write-Host "evidence: $Out"
$lines | Select-Object -Last 3
