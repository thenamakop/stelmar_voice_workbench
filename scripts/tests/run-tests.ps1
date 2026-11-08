# Runs every scripts/tests/*.Tests.ps1 in its own Windows PowerShell process.
# Run: powershell -NoProfile -ExecutionPolicy Bypass -File scripts/tests/run-tests.ps1
# Exit code 1 when any test file fails. ASCII only; Windows PowerShell 5.1.
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$failed = @()
$files = @(Get-ChildItem -Path $here -Filter '*.Tests.ps1' | Sort-Object Name)
foreach ($f in $files) {
  Write-Host "== $($f.Name)"
  & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $f.FullName
  if ($LASTEXITCODE -ne 0) { $failed += $f.Name }
}
Write-Host ""
if ($failed.Count) {
  Write-Host "Script tests failed: $($failed -join ', ')"
  exit 1
}
Write-Host "Script tests: all $($files.Count) file(s) passed"
