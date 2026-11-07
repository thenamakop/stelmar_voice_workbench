<#
.SYNOPSIS
  Fixes the discrepancies found in .setup\report.txt on 8 Nov 2026 (versions checked against the web that day).

.DESCRIPTION
  Run from a NORMAL (not administrator) PowerShell window in the repository folder. Each step asks
  before changing anything (Enter = yes). Installers that need administrator rights ask for approval
  themselves. Uses winget, which already manages most of these tools on this machine, so nothing is
  installed twice by a second package manager.

  Steps:
    1. Node.js: install 26 LTS with fnm (TripSync keeps 24); set up fnm --use-on-cd.
    2. pnpm: replace with the standalone winget package so it works with every Node version.
    1b. Remove the end-of-life Node.js 25.x install outside fnm.
    3. LLVM: put clang on PATH and set LIBCLANG_PATH (installed, but not on PATH).
    4. Visual Studio 2026 Build Tools with the C++ workload (only 2019 is installed).
    5. Python 3.14: update to the latest patch release.
    6. CMake: update if winget has the newer release.
    7. Vulkan SDK (Phase 2).
    8. CUDA Toolkit (Phase 2; after step 4).
    9. Re-run the setup report.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File .\scripts\fix-legion.ps1
#>
param([switch]$Yes)

$ErrorActionPreference = 'Continue'
$repo = Split-Path -Parent $PSScriptRoot
Set-Location $repo

$principal = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
if ($principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
  Write-Host "Run this from a normal PowerShell window, not as administrator." -ForegroundColor Yellow
  exit 1
}

function Update-SessionPath {
  # Adds new Machine and User PATH entries without dropping per-shell entries (such as fnm's).
  $current = @($env:Path -split ';' | Where-Object { $_ })
  $fresh = @(([Environment]::GetEnvironmentVariable('Path', 'Machine') + ';' + [Environment]::GetEnvironmentVariable('Path', 'User')) -split ';' | Where-Object { $_ })
  $env:Path = (@($current) + @($fresh | Where-Object { $current -notcontains $_ })) -join ';'
}
function Get-Exe([string]$name) {
  $c = Get-Command -Name $name -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
  if ($c) { return $c.Source }
  return $null
}
function Confirm-Step([string]$question) {
  if ($Yes) { return $true }
  $a = Read-Host ($question + " [Y/n]")
  return (-not $a) -or ($a -match '^[Yy]')
}
function Write-Step([string]$text) { Write-Host ""; Write-Host ("== " + $text + " ==") -ForegroundColor Cyan }

$winget = Get-Exe 'winget'
if (-not $winget) { Write-Host "winget not found. Install 'App Installer' from the Microsoft Store, then run again." -ForegroundColor Red; exit 1 }
function Invoke-Winget([string[]]$wargs) {
  Write-Host ("  winget " + ($wargs -join ' '))
  & $winget @wargs --accept-source-agreements --accept-package-agreements --disable-interactivity
}

# 1. Node.js (managed by fnm on this machine)
Write-Step "1. Node.js: use fnm for every project; remove the end-of-life 25.x install outside fnm"
Write-Host "  Copies on PATH right now:"
foreach ($n in @('node', 'npm', 'pnpm', 'claude', 'fnm')) {
  & where.exe $n 2>$null | ForEach-Object { Write-Host ("    " + $n + ": " + $_) }
}
$fnm = Get-Exe 'fnm'
if ($fnm) {
  if (Confirm-Step "  Install Node.js 26 LTS with fnm (TripSync keeps 24)?") {
    & $fnm install 26
    & $fnm list
  }
  Write-Host "  This repository pins Node 26 in .node-version; put '24' in TripSync's .node-version so fnm switches per folder."
  Write-Host "  Your PowerShell profile needs: fnm env --use-on-cd --shell powershell | Out-String | Invoke-Expression"
  $profileText = if (Test-Path $PROFILE) { Get-Content $PROFILE -Raw } else { '' }
  if ($profileText -notmatch 'use-on-cd') {
    if (Confirm-Step ("  Add that line to " + $PROFILE + "?")) {
      New-Item -ItemType File -Force -Path $PROFILE | Out-Null
      Add-Content -Path $PROFILE -Value 'fnm env --use-on-cd --shell powershell | Out-String | Invoke-Expression'
    }
  } else { Write-Host "  Profile already uses --use-on-cd." }
} else {
  Write-Host "  fnm not found on PATH. Install it (winget install Schniz.fnm) or run this from a shell where fnm is set up." -ForegroundColor Yellow
}

# 2. pnpm (standalone, so it works with every Node version fnm switches to)
Write-Step "2. pnpm (found 10.33.2; latest 12.18.0): standalone install independent of Node versions"
$pnpmNow = Get-Exe 'pnpm'
if ($pnpmNow) { Write-Host ("  Current pnpm: " + $pnpmNow) }
if (Confirm-Step "  Replace it with the standalone pnpm from winget (pnpm.pnpm)?") {
  if ($pnpmNow -and ($pnpmNow -match '\\npm\\|nodejs|fnm')) {
    $npm = Get-Exe 'npm'
    if ($npm) { Write-Host "  Removing the npm-installed pnpm first"; & $npm uninstall -g pnpm }
  }
  Invoke-Winget @('install', '--id', 'pnpm.pnpm', '--exact', '--silent')
  Update-SessionPath
  $p = Get-Exe 'pnpm'
  if ($p) { Write-Host ("  pnpm now: " + $p + " " + (& $p --version)) }
}

# 1b. Remove Node 25 outside fnm (after pnpm no longer depends on it)
Write-Step "1b. Remove the Node.js 25.x install outside fnm (end-of-life)"
$claudePath = Get-Exe 'claude'
if ($claudePath -and ($claudePath -match '\\npm\\|nodejs')) {
  Write-Host ("  Claude Code runs from " + $claudePath + ", which depends on that Node. Reinstall it with the native installer first: https://code.claude.com/docs/en/setup") -ForegroundColor Yellow
  Write-Host "  Skipping the removal for now." -ForegroundColor Yellow
} else {
  & $winget list --name "Node.js" --accept-source-agreements --disable-interactivity
  if (Confirm-Step "  Uninstall the Node.js 25.x package (OpenJS.NodeJS)? fnm's versions are not touched.") {
    Invoke-Winget @('uninstall', '--id', 'OpenJS.NodeJS', '--exact', '--silent')
    Write-Host "  If winget can't find it, remove 'Node.js' (version 25.x) in Settings > Apps > Installed apps."
  }
}

# 3. LLVM on PATH
Write-Step "3. LLVM (installed by winget, but clang is not on PATH; bindgen needs LIBCLANG_PATH)"
$llvmBin = Join-Path $env:ProgramFiles 'LLVM\bin'
if (-not (Test-Path (Join-Path $llvmBin 'clang.exe'))) {
  if (Confirm-Step "  LLVM not found in Program Files. Install it with winget?") {
    Invoke-Winget @('install', '--id', 'LLVM.LLVM', '--exact', '--silent')
  }
}
if (Test-Path (Join-Path $llvmBin 'clang.exe')) {
  if (Confirm-Step ("  Add " + $llvmBin + " to your user PATH and set LIBCLANG_PATH?")) {
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    if (-not (($userPath -split ';') -contains $llvmBin)) {
      [Environment]::SetEnvironmentVariable('Path', ($userPath.TrimEnd(';') + ';' + $llvmBin), 'User')
    }
    [Environment]::SetEnvironmentVariable('LIBCLANG_PATH', $llvmBin, 'User')
    Update-SessionPath
    Write-Host ("  " + (& (Join-Path $llvmBin 'clang.exe') --version | Select-Object -First 1))
  }
} else { Write-Host "  clang.exe still not found; install LLVM by hand from https://github.com/llvm/llvm-project/releases" -ForegroundColor Yellow }

# 4. Visual Studio 2026 Build Tools
Write-Step "4. Visual Studio Build Tools (found 2019 16.11; latest 2026 18.x)"
Write-Host "  The 2019 tools still build Rust and CUDA, but MSVC 2019 has no C++20 and is long out of mainstream support."
if (Confirm-Step "  Install Visual Studio 2026 Build Tools with the C++ workload (10 to 20 minutes, needs approval)?") {
  Invoke-Winget @('install', '--id', 'Microsoft.VisualStudio.BuildTools', '--exact', '--override', '--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended')
  Write-Host "  Keep 2019 for now; remove it later from Visual Studio Installer once Phase 2 builds cleanly with 2026."
}

# 5. Python
Write-Step "5. Python (found 3.14.3; latest 3.14 patch is 3.14.10)"
if (Confirm-Step "  Update Python 3.14 with winget?") {
  Invoke-Winget @('upgrade', '--id', 'Python.Python.3.14', '--exact', '--silent')
  Write-Host "  If winget says it isn't installed, update from https://www.python.org/downloads/ (or 'py install 3.14' with the Python install manager)."
}

# 6. CMake
Write-Step "6. CMake (found 4.4.4; latest 4.4.5)"
if (Confirm-Step "  Try to update CMake with winget?") {
  Invoke-Winget @('upgrade', '--id', 'Kitware.CMake', '--exact', '--silent')
  Write-Host "  If winget doesn't have 4.4.5 yet, this can wait: 4.4.4 is fine for Phase 2."
}

# 7. Vulkan SDK
Write-Step "7. Vulkan SDK (missing; latest 1.4.363.1; needed by Phase 2)"
if (-not $env:VULKAN_SDK) {
  if (Confirm-Step "  Install the Vulkan SDK with winget?") {
    Invoke-Winget @('install', '--id', 'KhronosGroup.VulkanSDK', '--exact', '--silent')
  }
} else { Write-Host ("  Already installed: " + $env:VULKAN_SDK) }

# 8. CUDA Toolkit
Write-Step "8. CUDA Toolkit (missing; latest 13.5.0; needed by Phase 2)"
Write-Host "  Install after the Visual Studio Build Tools so CUDA registers its build integration with them."
Write-Host "  Phase 2 checks that the whisper.cpp version bundled by whisper-rs supports this CUDA release."
if (-not (Get-Exe 'nvcc')) {
  if (Confirm-Step "  Install the CUDA Toolkit with winget (about 3 GB)?") {
    Invoke-Winget @('install', '--id', 'Nvidia.CUDA', '--exact', '--silent')
  }
} else { Write-Host "  nvcc already on PATH." }

# 9. Refresh the report
Write-Step "9. Refresh the setup report"
Write-Host "  Close this window and open a NEW normal PowerShell window (so PATH changes apply), then run:"
Write-Host "    powershell -ExecutionPolicy Bypass -File .\scripts\setup-legion.ps1 -NoGit"
Write-Host ""
Write-Host "Drivers are not changed by this script; update them by hand (see the links in docs\SETUP.md)." -ForegroundColor Yellow
