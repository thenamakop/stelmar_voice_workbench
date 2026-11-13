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
Test-Case 'build-engines.ps1 prints the cargo log tail and the whisper-rs-sys folders when the CMake cache is missing' {
  # The function is taken from the script's own syntax tree and run on a fake target folder.
  $path = Join-Path $repo 'scripts\build-engines.ps1'
  $ast = [System.Management.Automation.Language.Parser]::ParseFile($path, [ref]$null, [ref]$null)
  $fn = $ast.Find({ param($n) $n -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -eq 'Write-EngineBuildDiagnostics' }, $true)
  if (-not $fn) { throw 'Write-EngineBuildDiagnostics is not in build-engines.ps1' }
  $call = $ast.Find({ param($n) $n -is [System.Management.Automation.Language.CommandAst] -and $n.GetCommandName() -eq 'Write-EngineBuildDiagnostics' }, $true)
  if (-not $call) { throw 'the missing-cache failure does not call Write-EngineBuildDiagnostics' }
  $tmp = Join-Path ([IO.Path]::GetTempPath()) ('wb-eng-diag-' + [guid]::NewGuid().ToString('N'))
  try {
    $sys = Join-Path $tmp 'avx2\release\build\whisper-rs-sys-abc123'
    New-Item -ItemType Directory -Force -Path (Join-Path $sys 'out\lib') | Out-Null
    Set-Content -LiteralPath (Join-Path $sys 'output') -Value @('cargo:warning=configure failed', 'second line')
    $log = Join-Path $tmp 'avx2.build.log'
    Set-Content -LiteralPath $log -Value (1..300 | ForEach-Object { "log line $_" })
    Invoke-Expression $fn.Extent.Text
    $text = (Write-EngineBuildDiagnostics -Target (Join-Path $tmp 'avx2') -Log $log 6>&1 | ForEach-Object { "$_" }) -join "`n"
    foreach ($want in 'log line 300', 'whisper-rs-sys-abc123', 'out\ exists: True', 'out\build\ exists: False', 'CMakeCache.txt anywhere under out\: 0', 'configure failed') {
      if ($text -notlike "*$want*") { throw "the diagnostics lack '$want'" }
    }
    if ($text -like '*log line 99*') { throw 'more than the last 200 lines were printed' }
    if ($text -notlike '*log line 101*') { throw 'the last 200 lines are not all there' }
  } finally { Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue }
}
Test-Case 'build-engines.ps1 finds the whisper-rs-sys CMake cache in the old and the new cargo build layout' {
  $path = Join-Path $repo 'scripts\build-engines.ps1'
  $ast = [System.Management.Automation.Language.Parser]::ParseFile($path, [ref]$null, [ref]$null)
  $fn = $ast.Find({ param($n) $n -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -eq 'Find-WhisperSysBuildDir' }, $true)
  if (-not $fn) { throw 'Find-WhisperSysBuildDir is not in build-engines.ps1' }
  Invoke-Expression $fn.Extent.Text
  $tmp = Join-Path ([IO.Path]::GetTempPath()) ('wb-eng-find-' + [guid]::NewGuid().ToString('N'))
  try {
    # Before Rust 1.100: release\build\whisper-rs-sys-<hash>\out\build\CMakeCache.txt
    $old = Join-Path $tmp 'old\release\build\whisper-rs-sys-abc123'
    New-Item -ItemType Directory -Force -Path (Join-Path $old 'out\build') | Out-Null
    Set-Content -LiteralPath (Join-Path $old 'out\build\CMakeCache.txt') -Value 'x'
    # a compile-only folder of the same package (no cache) and another package with a cache
    New-Item -ItemType Directory -Force -Path (Join-Path $tmp 'old\release\build\whisper-rs-sys-def456\out') | Out-Null
    New-Item -ItemType Directory -Force -Path (Join-Path $tmp 'old\release\build\other-sys-111\out\build') | Out-Null
    Set-Content -LiteralPath (Join-Path $tmp 'old\release\build\other-sys-111\out\build\CMakeCache.txt') -Value 'x'
    $r = Find-WhisperSysBuildDir -Target (Join-Path $tmp 'old')
    if (-not $r -or $r.Name -ne 'whisper-rs-sys-abc123') { throw "old layout: got '$($r.FullName)'" }
    # The layout of the newer cargo: release\build\whisper-rs-sys\<hash>\out\build\CMakeCache.txt
    $new = Join-Path $tmp 'new\release\build\whisper-rs-sys\9f8e7d'
    New-Item -ItemType Directory -Force -Path (Join-Path $new 'out\build\ggml\src\ggml-vulkan') | Out-Null
    Set-Content -LiteralPath (Join-Path $new 'out\build\CMakeCache.txt') -Value 'x'
    # a nested project's own cache must not be taken for the top-level one
    Set-Content -LiteralPath (Join-Path $new 'out\build\ggml\src\ggml-vulkan\CMakeCache.txt') -Value 'x'
    $r = Find-WhisperSysBuildDir -Target (Join-Path $tmp 'new')
    if (-not $r -or $r.Name -ne '9f8e7d') { throw "new layout: got '$($r.FullName)'" }
    if (-not (Test-Path -LiteralPath (Join-Path $r.FullName 'out\build\CMakeCache.txt'))) { throw 'the folder returned does not hold out\build\CMakeCache.txt' }
    # nothing built
    if (Find-WhisperSysBuildDir -Target (Join-Path $tmp 'none')) { throw 'found something in a folder that does not exist' }
    New-Item -ItemType Directory -Force -Path (Join-Path $tmp 'empty\release\build') | Out-Null
    if (Find-WhisperSysBuildDir -Target (Join-Path $tmp 'empty')) { throw 'found something in an empty build folder' }
  } finally { Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue }
}
if ($script:failures) { Write-Host "$($script:failures) case(s) failed"; exit 1 }
Write-Host 'All cases passed'
