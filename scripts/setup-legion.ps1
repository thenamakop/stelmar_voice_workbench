<#
.SYNOPSIS
  Prepares the Legion (build machine) for the STELMAR Voice Workbench and initialises the Git repository.

.DESCRIPTION
  Run it from a NORMAL (not administrator) PowerShell window in the repository folder.
  Installers that need administrator rights ask for approval themselves. Running the whole
  script elevated would make the .git folder owned by Administrators, and Git would then
  refuse to work in it from your normal account ("dubious ownership").

  1. Checks every build tool the phases need and reports its version.
  2. -Update: upgrades installed tools that are outdated (winget, rustup, pnpm, Claude Code).
  3. -InstallMissing: installs missing Phase 0 tools with winget.
  4. -IncludePhase2: also checks or installs the Vulkan SDK and the CUDA Toolkit.
  5. Initialises Git (default branch master), enables .githooks, sets origin and makes the first
     commit with your own Git identity. Skip with -NoGit; skip the tool steps with -NoTools.
  Writes a report to .setup\report.txt (git-ignored).

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File .\scripts\setup-legion.ps1 -Update -InstallMissing
#>
param(
  [switch]$Update,
  [switch]$InstallMissing,
  [switch]$IncludePhase2,
  [switch]$NoGit,
  [switch]$NoTools,
  [string]$Origin = 'https://github.com/thenamakop/stelmar_voice_workbench.git'
)

$ErrorActionPreference = 'Continue'
$repo = Split-Path -Parent $PSScriptRoot
Set-Location $repo

# ---------- Refuse to run elevated ----------
$principal = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
if ($principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
  Write-Host "This window is running as administrator." -ForegroundColor Yellow
  Write-Host "Close it and run the script from a normal PowerShell window; installers will ask for approval themselves." -ForegroundColor Yellow
  exit 1
}

New-Item -ItemType Directory -Force -Path (Join-Path $repo '.setup') | Out-Null
$reportPath = Join-Path $repo '.setup\report.txt'
$report = New-Object System.Collections.Generic.List[string]

function Write-Report([string]$line) { Write-Host $line; $report.Add($line) }

function Update-SessionPath {
  # Adds new Machine and User PATH entries without dropping per-shell entries (such as fnm's).
  $current = @($env:Path -split ';' | Where-Object { $_ })
  $fresh = @(([Environment]::GetEnvironmentVariable('Path', 'Machine') + ';' + [Environment]::GetEnvironmentVariable('Path', 'User')) -split ';' | Where-Object { $_ })
  $env:Path = (@($current) + @($fresh | Where-Object { $current -notcontains $_ })) -join ';'
}

# Finds an executable on PATH only (never a function or alias), so names cannot collide.
function Get-Exe([string]$name) {
  $c = Get-Command -Name $name -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
  if ($c) { return $c.Source }
  return $null
}

function Get-FirstLine([string]$exe, [string[]]$exeArgs) {
  $path = Get-Exe $exe
  if (-not $path) { return $null }
  try {
    $out = & $path @exeArgs 2>&1 | Where-Object { "$_".Trim() } | Select-Object -First 1
    if ($out) { return "$out".Trim() }
  } catch {}
  return $null
}

$wingetExe = Get-Exe 'winget'

function Invoke-WingetPackage([string]$verb, [string]$id, [string]$override = '') {
  if (-not $wingetExe) { Write-Report "    winget not found; cannot $verb $id"; return }
  $wargs = @($verb, '--id', $id, '--exact', '--silent', '--accept-source-agreements', '--accept-package-agreements', '--disable-interactivity')
  if ($override) { $wargs += @('--override', $override) }
  Write-Report "    winget $verb $id ..."
  $out = & $wingetExe @wargs 2>&1 | ForEach-Object { "$_" } | Where-Object { $_.Trim() -and ($_ -notmatch '^[\s\-\\|/]+$') -and ($_ -notmatch '[\u2580-\u259F]') }
  $tail = ($out | Select-Object -Last 2) -join ' / '
  if ($tail) { Write-Report ("      " + $tail.Trim()) }
}

Write-Report "STELMAR Voice Workbench: Legion setup report"
Write-Report ("Date: " + (Get-Date -Format 'yyyy-MM-dd HH:mm'))
Write-Report ("Machine: " + $env:COMPUTERNAME + " | Windows " + [Environment]::OSVersion.Version.ToString())
Write-Report ("Switches: Update=" + $Update + " InstallMissing=" + $InstallMissing + " IncludePhase2=" + $IncludePhase2 + " NoGit=" + $NoGit + " NoTools=" + $NoTools)
Write-Report ("winget: " + $(if ($wingetExe) { $wingetExe } else { 'not found' }))
Write-Report ""

if (-not $NoTools) {
  # ---------- winget-managed tools ----------
  $tools = @(
    @{ Name = 'Git';          Exe = 'git';   Args = @('--version'); Id = 'Git.Git' },
    @{ Name = 'Rustup';       Exe = 'rustup';Args = @('--version'); Id = 'Rustlang.Rustup' },
    @{ Name = 'Node.js LTS';  Exe = 'node';  Args = @('--version'); Id = 'OpenJS.NodeJS.LTS' },
    @{ Name = 'CMake';        Exe = 'cmake'; Args = @('--version'); Id = 'Kitware.CMake' },
    @{ Name = 'LLVM (clang)'; Exe = 'clang'; Args = @('--version'); Id = 'LLVM.LLVM' },
    @{ Name = 'Python';       Exe = 'py';    Args = @('--version'); Id = 'Python.Python.3.12' },
    @{ Name = 'GitHub CLI';   Exe = 'gh';    Args = @('--version'); Id = 'GitHub.cli' }
  )

  Write-Report "== Tools =="
  foreach ($t in $tools) {
    if (Get-Exe $t.Exe) {
      Write-Report ("[found]   " + $t.Name + ": " + (Get-FirstLine $t.Exe $t.Args))
      if ($Update) { Invoke-WingetPackage 'upgrade' $t.Id }
    } else {
      Write-Report ("[missing] " + $t.Name)
      if ($InstallMissing) { Invoke-WingetPackage 'install' $t.Id; Update-SessionPath }
    }
  }
  Update-SessionPath

  # ---------- Rust toolchain ----------
  $rustup = Get-Exe 'rustup'
  if ($rustup -and ($Update -or $InstallMissing)) {
    Write-Report "    rustup: install stable, set default, update"
    & $rustup toolchain install stable 2>&1 | Out-Null
    & $rustup default stable 2>&1 | Out-Null
    & $rustup update 2>&1 | Out-Null
  }

  # ---------- pnpm ----------
  $pnpm = Get-Exe 'pnpm'
  if ($pnpm) {
    if ($Update) { Write-Report "    pnpm self-update"; & $pnpm self-update 2>&1 | Out-Null }
  } elseif ($InstallMissing) {
    $npm = Get-Exe 'npm'
    if ($npm) { Write-Report "    npm install -g pnpm"; & $npm install -g pnpm 2>&1 | Out-Null; Update-SessionPath }
  }

  # ---------- Claude Code ----------
  $claude = Get-Exe 'claude'
  if ($claude -and $Update) { Write-Report "    claude update"; & $claude update 2>&1 | Out-Null }

  # ---------- Visual Studio Build Tools (C++) ----------
  $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
  $vcName = $null; $vcVer = $null
  if (Test-Path $vswhere) {
    $vcName = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property displayName 2>$null
    $vcVer  = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationVersion 2>$null
  }
  if (-not $vcName -and $InstallMissing) {
    Write-Report "[missing] Visual Studio Build Tools with the C++ workload"
    Invoke-WingetPackage 'install' 'Microsoft.VisualStudio.2022.BuildTools' '--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended'
    if (Test-Path $vswhere) {
      $vcName = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property displayName 2>$null
      $vcVer  = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationVersion 2>$null
    }
  }

  # ---------- Phase 2 SDKs ----------
  if ($IncludePhase2) {
    if (-not $env:VULKAN_SDK -and $InstallMissing) { Invoke-WingetPackage 'install' 'KhronosGroup.VulkanSDK' }
    elseif ($env:VULKAN_SDK -and $Update) { Invoke-WingetPackage 'upgrade' 'KhronosGroup.VulkanSDK' }
    if (-not (Get-Exe 'nvcc') -and $InstallMissing) { Invoke-WingetPackage 'install' 'Nvidia.CUDA' }
  }
  Update-SessionPath

  # ---------- Final versions ----------
  Write-Report ""
  Write-Report "== Versions after this run =="
  $checks = @(
    @('Git', 'git', @('--version')), @('rustup', 'rustup', @('--version')), @('rustc', 'rustc', @('--version')),
    @('cargo', 'cargo', @('--version')), @('Node.js', 'node', @('--version')), @('npm', 'npm', @('--version')),
    @('pnpm', 'pnpm', @('--version')), @('CMake', 'cmake', @('--version')), @('clang (LLVM)', 'clang', @('--version')),
    @('Python', 'py', @('--version')), @('GitHub CLI', 'gh', @('--version')), @('Claude Code', 'claude', @('--version')),
    @('nvcc (CUDA)', 'nvcc', @('--version'))
  )
  foreach ($c in $checks) {
    $v = Get-FirstLine $c[1] $c[2]
    if ($c[1] -eq 'nvcc' -and $v) { $v = (& (Get-Exe 'nvcc') --version 2>&1 | Select-String 'release' | Select-Object -First 1) -as [string] }
    if ($v) { Write-Report ("[ok]      " + $c[0] + ": " + $v.Trim()) } else { Write-Report ("[missing] " + $c[0]) }
  }
  if ($vcName) { Write-Report ("[ok]      MSVC C++ tools: " + $vcName + " " + $vcVer) } else { Write-Report "[missing] Visual Studio Build Tools with the C++ workload" }
  if ($env:VULKAN_SDK) { Write-Report ("[ok]      Vulkan SDK: " + $env:VULKAN_SDK) } else { Write-Report "[missing] Vulkan SDK (Phase 2; rerun with -IncludePhase2)" }

  $wv = $null
  foreach ($k in @('HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}',
                   'HKCU:\Software\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}')) {
    if (-not $wv -and (Test-Path $k)) { $wv = (Get-ItemProperty $k -ErrorAction SilentlyContinue).pv }
  }
  if ($wv) { Write-Report ("[ok]      WebView2 runtime: " + $wv) } else { Write-Report "[missing] WebView2 runtime" }

  # ---------- Hardware ----------
  Write-Report ""
  Write-Report "== Hardware =="
  $gpus = @(Get-CimInstance Win32_VideoController -ErrorAction SilentlyContinue)
  foreach ($g in $gpus) { Write-Report ("GPU: " + $g.Name + " | driver " + $g.DriverVersion) }
  if (-not ($gpus | Where-Object { $_.Name -match 'Intel' })) {
    Write-Report "WARNING: the Intel integrated GPU is not visible. Set GPU mode to Hybrid in Lenovo Vantage (or the BIOS) and reboot; emulation needs it."
  }
  $cable = Get-PnpDevice -ErrorAction SilentlyContinue | Where-Object { $_.FriendlyName -match 'VB-Audio|CABLE Input|CABLE Output' }
  if ($cable) { Write-Report "[ok]      VB-CABLE virtual audio cable" } else { Write-Report "[missing] VB-CABLE (install by hand from vb-audio.com before Phase 1; reboot after)" }
  $ramGb = [math]::Round((Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory / 1GB, 1)
  Write-Report ("RAM: " + $ramGb + " GB | Logical processors: " + [Environment]::ProcessorCount)
}

# ---------- Git ----------
if (-not $NoGit) {
  Write-Report ""
  Write-Report "== Git =="
  Update-SessionPath
  $git = Get-Exe 'git'
  if (-not $git) { Write-Report "Git is missing; skipping repository setup." }
  else {
    if (-not (Test-Path (Join-Path $repo '.git'))) {
      & $git init -b master 2>&1 | Out-Null
      Write-Report "Initialised repository (default branch master)."
    } else { Write-Report "Repository already exists." }
    & $git config core.hooksPath .githooks
    Write-Report "Enabled .githooks (commit-msg strips AI attribution)."
    $current = & $git remote get-url origin 2>$null
    if (-not $current) { & $git remote add origin $Origin; Write-Report ("Added origin " + $Origin) }
    elseif ($current -ne $Origin) { & $git remote set-url origin $Origin; Write-Report ("Changed origin to " + $Origin) }
    else { Write-Report ("Origin already " + $Origin) }

    $name = & $git config user.name
    $email = & $git config user.email
    if (-not $name -or -not $email) {
      Write-Report "Git identity not set. Set user.name and user.email, then run again with -NoTools."
    } else {
      Write-Report ("Commit identity: " + $name + " <" + $email + ">")
      foreach ($p in @('.claude\settings.json', '.github\workflows\no-ai-attribution.yml', '.claude\skills\run-phase\SKILL.md')) {
        if (-not (Test-Path (Join-Path $repo $p))) { Write-Report ("WARNING: " + $p + " is missing; copy the .claude and .github folders from the kit zip first.") }
      }
      & $git add -A
      $pending = & $git status --porcelain
      $hasHead = $true
      & $git rev-parse --verify HEAD 2>$null | Out-Null
      if ($LASTEXITCODE -ne 0) { $hasHead = $false }
      if ($pending -and -not $hasHead) {
        & $git commit -q -m "Initial commit: STELMAR Voice Workbench starter kit" -m "Plan, agent rules, phase prompts, design system, mockups and setup scripts."
        Write-Report ("Committed: " + (& $git log -1 --format='%h %an <%ae> %s'))
        Write-Report ("Files in commit: " + (& $git ls-files | Measure-Object).Count)
      } elseif ($pending) {
        Write-Report "Repository already has commits; staged changes left for you to commit."
      } else { Write-Report "Nothing to commit." }
      Write-Report "Not pushed. When ready: git push -u origin master"
    }
  }
}

Write-Report ""
Write-Report "Done. Report saved to .setup\report.txt"
$report | Out-File -FilePath $reportPath -Encoding utf8
