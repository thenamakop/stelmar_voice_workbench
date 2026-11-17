# A lint for "The term 'X' is not recognized" in run-gates.ps1. Twice the runner called a function that its own scope
# could not see (Banner in a closure on 15 Nov; Invoke-PackNative, which PackCore does not export, in the restart step on
# 17 Nov). Every command the runner calls by a Verb-Noun name must be a function the runner defines, a function one of
# the modules it imports exports, or a command of Windows PowerShell 5.1.
# Run: powershell -NoProfile -ExecutionPolicy Bypass -File scripts/tests/RunnerCommands.Tests.ps1
# ASCII only; Windows PowerShell 5.1.
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$root = (Resolve-Path (Join-Path $here '..')).Path
$fails = 0
function Check([string]$Name, [bool]$Ok) {
  if ($Ok) { Write-Host "ok   $Name" } else { Write-Host "FAIL $Name"; $script:fails++ }
}
# The runner starts with a clean module path (Git Bash leaks its own); so does this test.
Import-Module (Join-Path $root 'lib\PsEnv.psm1') -Force
Set-WbCleanPSModulePath
$runner = Join-Path $root 'gate-pack\run-gates.ps1'
$t = $null; $e = $null
$ast = [System.Management.Automation.Language.Parser]::ParseFile($runner, [ref]$t, [ref]$e)
Check 'run-gates.ps1 parses' (@($e).Count -eq 0)

# The modules the runner imports (the lines "Import-Module (Join-Path $libDir 'X.psm1')").
$text = Get-Content -LiteralPath $runner -Raw
$mods = @([regex]::Matches($text, "Import-Module \(Join-Path \`$libDir '([A-Za-z]+\.psm1)'\)") | ForEach-Object { $_.Groups[1].Value } | Sort-Object -Unique)
Check ('the runner imports modules: ' + ($mods -join ', ')) ($mods.Count -ge 3)
$exported = @{}
foreach ($m in $mods) {
  $mod = Import-Module (Join-Path $root "lib\$m") -Force -PassThru
  foreach ($k in $mod.ExportedCommands.Keys) { $exported[$k] = $m }
}
$defined = @{}
foreach ($f in @($ast.FindAll({ param($n) $n -is [System.Management.Automation.Language.FunctionDefinitionAst] }, $true))) { $defined[$f.Name] = $true }

$bad = New-Object System.Collections.Generic.List[string]
$seen = @{}
foreach ($c in @($ast.FindAll({ param($n) $n -is [System.Management.Automation.Language.CommandAst] }, $true))) {
  $name = $c.GetCommandName()
  if (-not $name -or $name -notmatch '^[A-Za-z]+-[A-Za-z]+$' -or $seen.ContainsKey($name)) { continue }
  $seen[$name] = $true
  if ($defined.ContainsKey($name) -or $exported.ContainsKey($name)) { continue }
  # A command of Windows PowerShell itself (Get-ChildItem, Join-Path, ...) or of its standard modules.
  $cmd = Get-Command $name -ErrorAction SilentlyContinue
  if ($cmd -and $cmd.Source -notmatch 'Pack|Wb') { continue }
  $bad.Add("$name (line $($c.Extent.StartLineNumber))")
}
Check ('every command the runner calls is defined, exported by an imported module or built in (' + $seen.Count + ' checked)') ($bad.Count -eq 0)
if ($bad.Count -gt 0) { $bad | ForEach-Object { Write-Host "     not visible to the runner: $_" } }

if ($fails) { Write-Host "$fails failed"; exit 1 }
Write-Host 'RunnerCommands tests passed'
