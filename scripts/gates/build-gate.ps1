<#
.SYNOPSIS
  Builds what the Phase 1 gates run: the UI, the gate build of the app and the release builds of
  wb-testbench and wb-emulate (development only).

.DESCRIPTION
  The gate build is target\gate\stelmar-voice-workbench.exe: cargo profile "gate" (optimised like
  release), features gate-hooks (environment-variable test hooks) and custom-protocol (serves the
  built UI from dist\). src-tauri\build.rs refuses gate-hooks in the release profile, so no
  release or installer build can contain the hooks.
#>
$ErrorActionPreference = 'Stop'
$repo = Resolve-Path (Join-Path (Split-Path -Parent $MyInvocation.MyCommand.Path) '..\..')
Push-Location $repo
try {
  pnpm build
  if ($LASTEXITCODE -ne 0) { throw 'pnpm build failed' }
  cargo build -p stelmar-voice-workbench --profile gate --features 'gate-hooks custom-protocol'
  if ($LASTEXITCODE -ne 0) { throw 'gate build failed' }
  cargo build --release -p wb-testbench -p wb-emulate
  if ($LASTEXITCODE -ne 0) { throw 'tool build failed' }
  Get-Item target\gate\stelmar-voice-workbench.exe, target\release\wb-testbench.exe, target\release\wb-emulate.exe |
    Select-Object Name, Length, LastWriteTime | Format-Table -AutoSize
} finally {
  Pop-Location
}
