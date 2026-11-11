# Tests that the cuda engine stays out of what ships (owner's decision of 11 Nov 2026: Vulkan only).
# Run: powershell -NoProfile -ExecutionPolicy Bypass -File scripts/tests/ReleaseEngines.Tests.ps1
# No Pester needed. Exit code 1 when any case fails. ASCII only; Windows PowerShell 5.1.
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo = (Resolve-Path (Join-Path $here '..\..')).Path
$script:failures = 0
function Test-Case([string]$Name, [scriptblock]$Body) {
  try { & $Body; Write-Host "PASS  $Name" } catch { $script:failures++; Write-Host "FAIL  $Name : $($_.Exception.Message)" }
}
function Get-DefaultVariants([string]$ScriptPath) {
  $ast = [System.Management.Automation.Language.Parser]::ParseFile($ScriptPath, [ref]$null, [ref]$null)
  $p = $ast.ParamBlock.Parameters | Where-Object { $_.Name.VariablePath.UserPath -eq 'Variants' }
  if (-not $p -or -not $p.DefaultValue) { throw "no Variants default in $ScriptPath" }
  $p.DefaultValue.Extent.Text
}

Test-Case 'the release bundle lists avx2, avx512 and vulkan, and no cuda' {
  $conf = Get-Content -Raw (Join-Path $repo 'src-tauri\tauri.engines.conf.json') | ConvertFrom-Json
  $bins = @($conf.bundle.externalBin)
  foreach ($v in 'avx2', 'avx512', 'vulkan') { if ($bins -notcontains "binaries/wb-engine-$v") { throw "missing $v" } }
  if (@($bins | Where-Object { $_ -match 'cuda' }).Count) { throw 'cuda is in the release bundle' }
}
Test-Case 'build-engines.ps1 and engines-only.ps1 (the gate pack) do not build or use cuda by default' {
  foreach ($f in 'scripts\build-engines.ps1', 'scripts\gates\engines-only.ps1') {
    $d = Get-DefaultVariants (Join-Path $repo $f)
    if ($d -match 'cuda') { throw "$f defaults to: $d" }
  }
}
Test-Case 'CI builds no cuda engine' {
  $ci = Get-Content -Raw (Join-Path $repo '.github\workflows\ci.yml')
  if ($ci -match '(?i)cuda') { throw 'ci.yml mentions cuda' }
}
if ($script:failures) { Write-Host "$($script:failures) case(s) failed"; exit 1 }
Write-Host 'All cases passed'
