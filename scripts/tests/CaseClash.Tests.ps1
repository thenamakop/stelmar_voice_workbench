# A lint for a mistake PowerShell makes easy: variable names ignore case, so `foreach ($l in ...)` overwrites a list
# called `$L` ("[System.String] does not contain a method named 'Add'"). It cost three 15-minute microphone runs.
# Every script and module of scripts\ (not the tests) is read: no foreach variable may share its name, apart from
# the case, with a variable assigned or a parameter declared in the same file.
# Run: powershell -NoProfile -ExecutionPolicy Bypass -File scripts/tests/CaseClash.Tests.ps1
# ASCII only; Windows PowerShell 5.1.
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$root = (Resolve-Path (Join-Path $here '..')).Path
$fails = 0
function Check([string]$Name, [bool]$Ok) {
  if ($Ok) { Write-Host "ok   $Name" } else { Write-Host "FAIL $Name"; $script:fails++ }
}

function Find-CaseClashes([string]$Path) {
  $t = $null; $e = $null
  $ast = [System.Management.Automation.Language.Parser]::ParseFile($Path, [ref]$t, [ref]$e)
  $hits = New-Object System.Collections.Generic.List[string]
  $fnType = [System.Management.Automation.Language.FunctionDefinitionAst]
  # The nearest enclosing function of a node, or $null for the script's own level.
  $ownerOf = { param($n) $p = $n.Parent; while ($p -and -not ($p -is $fnType)) { $p = $p.Parent }; $p }
  $scopes = @($null) + @($ast.FindAll({ param($n) $n -is [System.Management.Automation.Language.FunctionDefinitionAst] }, $true))
  foreach ($sc in $scopes) {
    $fe = @($ast.FindAll({ param($n) $n -is [System.Management.Automation.Language.ForEachStatementAst] }, $true) | Where-Object { [object]::ReferenceEquals((& $ownerOf $_), $sc) })
    $names = @{}
    foreach ($a in @($ast.FindAll({ param($n) $n -is [System.Management.Automation.Language.AssignmentStatementAst] -and $n.Left -is [System.Management.Automation.Language.VariableExpressionAst] }, $true) | Where-Object { [object]::ReferenceEquals((& $ownerOf $_), $sc) })) {
      $names[$a.Left.VariablePath.UserPath] = "assigned at line $($a.Extent.StartLineNumber)"
    }
    if ($null -ne $sc) {
      foreach ($p in @($sc.FindAll({ param($n) $n -is [System.Management.Automation.Language.ParameterAst] }, $true) | Where-Object { [object]::ReferenceEquals((& $ownerOf $_), $sc) })) {
        $names[$p.Name.VariablePath.UserPath] = "a parameter at line $($p.Extent.StartLineNumber)"
      }
    } else {
      foreach ($p in @($ast.FindAll({ param($n) $n -is [System.Management.Automation.Language.ParameterAst] }, $true) | Where-Object { $null -eq (& $ownerOf $_) })) {
        $names[$p.Name.VariablePath.UserPath] = "a parameter at line $($p.Extent.StartLineNumber)"
      }
    }
    foreach ($x in $fe) {
      $n = $x.Variable.VariablePath.UserPath
      foreach ($k in @($names.Keys)) {
        if ($k -ine $n -or $k -ceq $n) { continue }
        $hits.Add(('{0}:{1} foreach ${2} clashes with ${3} ({4})' -f (Split-Path $Path -Leaf), $x.Extent.StartLineNumber, $n, $k, $names[$k]))
      }
    }
  }
  @($hits | Sort-Object -Unique)
}

# The detector works: a file with the mistake is caught, the same file with another name is not.
$tmp = Join-Path ([IO.Path]::GetTempPath()) ('wb-clash-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path $tmp | Out-Null
try {
  $bad = Join-Path $tmp 'bad.ps1'
  Set-Content -LiteralPath $bad -Value @('$L = New-Object System.Collections.Generic.List[string]', 'foreach ($l in @(''a'', ''b'')) { $L.Add($l) }') -Encoding ASCII
  Check 'the lint finds a loop variable that differs from a list only by case' (@(Find-CaseClashes $bad).Count -eq 1)
  $badParam = Join-Path $tmp 'badparam.ps1'
  Set-Content -LiteralPath $badParam -Value @('function F { param($Lines) foreach ($lines in 1..3) { $lines } }') -Encoding ASCII
  Check 'the lint finds a loop variable that differs from a parameter only by case' (@(Find-CaseClashes $badParam).Count -ge 1)
  $good = Join-Path $tmp 'good.ps1'
  Set-Content -LiteralPath $good -Value @('$out = New-Object System.Collections.Generic.List[string]', 'foreach ($l in @(''a'', ''b'')) { $out.Add($l) }') -Encoding ASCII
  Check 'the lint passes the same loop with distinct names' (@(Find-CaseClashes $good).Count -eq 0)
} finally { Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue }

$all = New-Object System.Collections.Generic.List[string]
foreach ($f in @(Get-ChildItem -LiteralPath $root -Recurse -Include *.ps1, *.psm1 | Where-Object { $_.FullName -notmatch '\\tests\\' })) {
  foreach ($h in @(Find-CaseClashes $f.FullName)) { $all.Add($h) }
}
Check 'no script or module of scripts\ has a foreach variable that differs from another variable only by case' ($all.Count -eq 0)
foreach ($h in $all) { Write-Host "     $h" }

if ($fails) { Write-Host "$fails failed"; exit 1 }
Write-Host 'CaseClash tests passed'
