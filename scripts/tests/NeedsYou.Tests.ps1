# Tests that every message the pack shows to the person can really be printed: the "needs you" banner of
# a gate that asks something (network-off, rename-engines, mic-run), the restart banner, and every other
# banner of run-gates.ps1. A call to a function the running scope cannot see ("The term 'Banner' is not
# recognized") must fail here, not on the laptop.
# Run: powershell -NoProfile -ExecutionPolicy Bypass -File scripts/tests/NeedsYou.Tests.ps1
# ASCII only; Windows PowerShell 5.1.
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo = (Resolve-Path (Join-Path $here '..\..')).Path
$fails = 0
function Check([string]$Name, [bool]$Ok) {
  if ($Ok) { Write-Host "ok   $Name" } else { Write-Host "FAIL $Name"; $script:fails++ }
}
$ps = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
$runner = Join-Path $repo 'scripts\gate-pack\run-gates.ps1'
$gateDir = Join-Path $repo 'scripts\gate-pack\gates'
$manifest = Get-Content -LiteralPath (Join-Path $repo 'scripts\gate-pack\gates.manifest.json') -Raw -Encoding UTF8 | ConvertFrom-Json
Import-Module (Join-Path $repo 'scripts\lib\PackCore.psm1') -Force
Import-Module (Join-Path $repo 'scripts\lib\PackGate.psm1') -Force

# ---- 1. Static: every command the runner calls resolves, and no closure hides a runner function ----
$tokens = $null; $errs = $null
$ast = [System.Management.Automation.Language.Parser]::ParseFile($runner, [ref]$tokens, [ref]$errs)
Check 'run-gates.ps1 parses' (@($errs).Count -eq 0)
$defined = @($ast.FindAll({ param($n) $n -is [System.Management.Automation.Language.FunctionDefinitionAst] }, $true) | ForEach-Object { $_.Name })
# The modules the runner imports (the same ones it loads before it prints anything).
foreach ($m in 'PsEnv.psm1', 'PackCore.psm1', 'PackGate.psm1', 'CableRig.psm1', 'LoopbackRig.psm1') {
  $f = Join-Path $repo ('scripts\lib\' + $m)
  if (Test-Path -LiteralPath $f) { Import-Module $f -Force -ErrorAction SilentlyContinue }
}
$imports = @($ast.FindAll({ param($n) $n -is [System.Management.Automation.Language.CommandAst] }, $true) |
  Where-Object { $_.GetCommandName() -eq 'Import-Module' } | ForEach-Object { $_.Extent.Text })
$importedNames = @($imports | ForEach-Object { if ($_ -match "'([A-Za-z0-9]+\.psm1)'") { $Matches[1] } })
Check 'the runner imports PackCore (the module that holds the banner)' ($importedNames -contains 'PackCore.psm1')
$unresolved = New-Object System.Collections.Generic.List[string]
foreach ($c in @($ast.FindAll({ param($n) $n -is [System.Management.Automation.Language.CommandAst] }, $true))) {
  $name = $c.GetCommandName()
  if (-not $name) { continue }
  if ($defined -contains $name) { continue }
  if (-not (Get-Command -Name $name -ErrorAction SilentlyContinue)) { $unresolved.Add("$name (line $($c.Extent.StartLineNumber))") }
}
Check 'every command run-gates.ps1 calls is a cmdlet, a module function or a function it defines' ($unresolved.Count -eq 0)
foreach ($u in $unresolved) { Write-Host "     unresolved: $u" }
# A closure (or a script block given to the process helper) runs in the module's scope: it cannot call
# the functions run-gates.ps1 defines (Banner, Say, ...).
$closures = @($ast.FindAll({ param($n) $n -is [System.Management.Automation.Language.InvokeMemberExpressionAst] -and $n.Member.Value -eq 'GetNewClosure' }, $true))
Check 'run-gates.ps1 builds no closure (a closure cannot see its functions); handlers come from the module' ($closures.Count -eq 0)
$blockCalls = New-Object System.Collections.Generic.List[string]
foreach ($sb in @($ast.FindAll({ param($n) $n -is [System.Management.Automation.Language.ScriptBlockExpressionAst] }, $true))) {
  # Script blocks that are the body of a function or a plain argument inside the script scope are fine; only
  # a block given as -OnTick is run by the module.
  $parent = $sb.Parent
  if ($parent -is [System.Management.Automation.Language.CommandParameterAst] -or ($parent -is [System.Management.Automation.Language.CommandAst])) {
    $txt = $parent.Extent.Text
    if ($txt -match '-OnTick') {
      foreach ($cc in @($sb.FindAll({ param($n) $n -is [System.Management.Automation.Language.CommandAst] }, $true))) {
        if ($defined -contains $cc.GetCommandName()) { $blockCalls.Add($cc.GetCommandName()) }
      }
    }
  }
}
Check 'no script block given to -OnTick calls a function defined in run-gates.ps1' ($blockCalls.Count -eq 0)

# ---- 2. Every gate that asks the person something reaches the person ----
$interactive = @($manifest.gates | Where-Object { $_.interactive })
Check 'the interactive gates are network-off, rename-engines and mic-run' ((@($interactive | ForEach-Object { $_.id } | Sort-Object) -join ',') -eq 'mic-run,network-off,rename-engines')
foreach ($g in $interactive) {
  $script = Join-Path $repo ('scripts\gate-pack\' + ([string]$g.script -replace '/', '\'))
  $text = Get-Content -LiteralPath $script -Raw -Encoding UTF8
  Check "$($g.id): the gate asks through Request-PackPrompt (the runner's banner), not a console call of its own" ($text -match 'Request-PackPrompt' -and $text -notmatch 'Read-Host')
}

# The same mechanism as the runner uses, with each interactive gate's own id and a message of its own shape:
# a fake gate script writes PROMPT.txt through Request-PackPrompt and waits for the answer; the handler
# from the module shows the banner and answers.
$tmp = Join-Path ([IO.Path]::GetTempPath()) ('wb-needsyou-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path $tmp | Out-Null
try {
  $fake = Join-Path $tmp 'fake-gate.ps1'
  Set-Content -LiteralPath $fake -Encoding ASCII -Value @(
    'param([string]$CtxFile, [string]$ResultFile)',
    ('$lib = ''' + (Join-Path $repo 'scripts\lib') + ''''),
    'Import-Module (Join-Path $lib ''PackCore.psm1'') -Force; Import-Module (Join-Path $lib ''PackGate.psm1'') -Force',
    '$ctx = Read-PackCtx -CtxFile $CtxFile',
    '$a = Request-PackPrompt -Ctx $ctx -TimeoutMinutes 1 -Message ("FIRST LINE of the message`r`n  - second line`r`nPress Enter to start.")',
    '$r = New-PackResult -Id ([string]$ctx.gate_id) -Status "PASS" -Headline ("answer " + $a)',
    'Write-PackResult -Path $ResultFile -Result $r')
  foreach ($g in $interactive) {
    $gd = Join-Path $tmp $g.id
    New-Item -ItemType Directory -Force -Path $gd | Out-Null
    $ctxFile = Join-Path $gd 'ctx.json'
    Write-PackJson -Path $ctxFile -Value ([ordered]@{ gate_id = [string]$g.id; gate_dir = $gd; work_dir = $tmp; pack_root = $repo; profile = 'laptop'; quick = $false; machine = 'TEST' })
    $res = Join-Path $gd 'result.json'
    $log = Join-Path $tmp 'runner.log'
    $handler = New-PackPromptHandler -GateId ([string]$g.id) -GateDir $gd -LogFile $log -Read { param($p) 'typed-answer' }
    $out = @(Invoke-PackGateProcess -ScriptPath $fake -CtxFile $ctxFile -ResultFile $res -LogFile (Join-Path $gd 'gate.log') -TimeoutMinutes 2 -OnTick $handler 6>&1 4>&1 2>&1 | ForEach-Object { "$_" })
    $text = $out -join "`n"
    Check "$($g.id): the banner prints (title, message lines) and the person's answer reaches the gate" ($text -match "$($g.id) needs you:" -and $text -match 'FIRST LINE of the message' -and $text -match 'second line' -and $text -notmatch 'is not recognized')
    $r = Get-Content -LiteralPath $res -Raw -Encoding UTF8 | ConvertFrom-Json
    Check "$($g.id): the gate got the answer" ($r.headline -eq 'answer typed-answer')
    Check "$($g.id): the banner is also in the runner log" ((Get-Content -LiteralPath $log -Raw -Encoding UTF8) -match "$($g.id) needs you:")
  }
} finally { Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue }

# ---- 3. The real runner, attended, with a fake gate that asks ----
$tmp2 = Join-Path ([IO.Path]::GetTempPath()) ('wb-needsyou-run-' + [guid]::NewGuid().ToString('N'))
$pack = Join-Path $tmp2 'pack'
try {
  New-Item -ItemType Directory -Force -Path (Join-Path $pack 'scripts\gate-pack\gates'), (Join-Path $pack 'scripts\lib') | Out-Null
  Copy-Item -Path (Join-Path $repo 'scripts\lib\*') -Destination (Join-Path $pack 'scripts\lib') -Recurse -Force
  Copy-Item -LiteralPath $runner -Destination (Join-Path $pack 'run-gates.ps1')
  Set-Content -LiteralPath (Join-Path $pack 'scripts\gate-pack\gates\ask.ps1') -Encoding ASCII -Value @(
    'param([string]$CtxFile, [string]$ResultFile)',
    '$lib = Join-Path $PSScriptRoot ''..\..\lib''',
    'Import-Module (Join-Path $lib ''PackCore.psm1'') -Force; Import-Module (Join-Path $lib ''PackGate.psm1'') -Force',
    '$ctx = Read-PackCtx -CtxFile $CtxFile; Initialize-PackGate -Ctx $ctx',
    '$null = Request-PackPrompt -Ctx $ctx -TimeoutMinutes 1 -Message ("Switch the network OFF now:`r`n  - turn Wi-Fi off`r`nPress Enter when the network is off.")',
    '$r = New-PackResult -Id "ask" -Status "PASS" -Headline "asked" -Rig "fake rig" -Quick ([bool]$ctx.quick)',
    'Complete-PackGate -Ctx $ctx -Result $r -ResultFile $ResultFile')
  # A second interactive gate that ends INVALID after its prompt: the person must not be asked again in the same run.
  Set-Content -LiteralPath (Join-Path $pack 'scripts\gate-pack\gates\ask-bad.ps1') -Encoding ASCII -Value @(
    'param([string]$CtxFile, [string]$ResultFile)',
    '$lib = Join-Path $PSScriptRoot ''..\..\lib''',
    'Import-Module (Join-Path $lib ''PackCore.psm1'') -Force; Import-Module (Join-Path $lib ''PackGate.psm1'') -Force',
    '$ctx = Read-PackCtx -CtxFile $CtxFile; Initialize-PackGate -Ctx $ctx',
    '$null = Request-PackPrompt -Ctx $ctx -TimeoutMinutes 1 -Message ("SECOND GATE asks once`r`nPress Enter.")',
    '$r = New-PackResult -Id "ask-bad" -Status "INVALID" -Headline "invalid on purpose" -Quick ([bool]$ctx.quick)',
    'Complete-PackGate -Ctx $ctx -Result $r -ResultFile $ResultFile')
  $m = [ordered]@{ pack_version = 1; gates = @(
    [ordered]@{ id = 'ask'; title = 'fake ask'; priority = 'P1'; order = 1; script = 'gates/ask.ps1'; group = 'any'; needs = @{}; interactive = $true; est_minutes = [ordered]@{ low = 1; mid = 1 }; depends_on = @(); ledger = @() },
    [ordered]@{ id = 'ask-bad'; title = 'fake ask-bad'; priority = 'P1'; order = 2; script = 'gates/ask-bad.ps1'; group = 'any'; needs = @{}; interactive = $true; est_minutes = [ordered]@{ low = 1; mid = 1 }; depends_on = @(); ledger = @() }) }
  ($m | ConvertTo-Json -Depth 8) | Set-Content -LiteralPath (Join-Path $pack 'scripts\gate-pack\gates.manifest.json') -Encoding UTF8
  $factsFile = Join-Path $tmp2 'facts.json'
  Set-Content -LiteralPath $factsFile -Value '{"interface":true,"ac":true,"idle":true}' -Encoding ASCII
  # Started the way a person starts it from a console: the call operator (a script's functions are not in the
  # global scope there, which is what broke Banner inside the prompt handler), not -File.
  $cmd = "& '" + (Join-Path $pack 'run-gates.ps1') + "' -Role low -NoRestart -FactsFile '$factsFile' -WorkDir '" + (Join-Path $tmp2 'work') + "' -BackupRoot '" + (Join-Path $tmp2 'backup') + "' -LockDir '$tmp2' -NoZip"
  # Windows PowerShell 5.1 and, where it is installed, PowerShell 7: a person may start the pack from either.
  $hosts = @($ps)
  $pwsh = Get-Command pwsh -ErrorAction SilentlyContinue
  if ($pwsh) { $hosts += $pwsh.Source }
  foreach ($h in $hosts) {
    $hn = Split-Path -Leaf $h
    if (Test-Path -LiteralPath (Join-Path $tmp2 'work')) { Remove-Item -LiteralPath (Join-Path $tmp2 'work') -Recurse -Force }
    $eap = $ErrorActionPreference; $ErrorActionPreference = 'Continue'
    $out = @("`n`n`n`n`n" | & $h -NoProfile -ExecutionPolicy Bypass -Command $cmd 2>&1 | ForEach-Object { "$_" })
    $ErrorActionPreference = $eap
    $t = $out -join "`n"
    Check "$hn : the attended runner prints the needs-you banner of a gate that asks" ($t -match 'ask needs you:' -and $t -match 'Switch the network OFF now' -and $t -match 'Press Enter when the network is off')
    Check "$hn : the attended runner reports no unrecognized term" ($t -notmatch 'is not recognized')
    Check "$hn : the gate finished after the answer" ($t -match 'PASS\s+ask')
    Check "$hn : an interactive gate that ends INVALID asks the person once, not once per try" (([regex]::Matches($t, 'ask-bad needs you:')).Count -eq 1 -and $t -match 'INVALID\s+ask-bad')
  }
} finally { Remove-Item -LiteralPath $tmp2 -Recurse -Force -ErrorAction SilentlyContinue }

# ---- 4. The restart banner (Confirm-Restart) and the plain banner run in a scope like the runner's ----
$fnBanner = @($ast.FindAll({ param($n) $n -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -eq 'Banner' }, $true))[0]
$fnRestart = @($ast.FindAll({ param($n) $n -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -eq 'Confirm-Restart' }, $true))[0]
Check 'run-gates.ps1 defines Banner and Confirm-Restart' ($null -ne $fnBanner -and $null -ne $fnRestart)
$WorkDir = Join-Path ([IO.Path]::GetTempPath()) ('wb-needsyou-wd-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path $WorkDir | Out-Null
try {
  Invoke-Expression $fnBanner.Extent.Text
  Invoke-Expression $fnRestart.Extent.Text
  function Read-Host { param($Prompt) 'RESTART' }
  $o = @(& { $script:answer = Confirm-Restart -For @('hashing-1', 'load-cold') } 6>&1 4>&1 2>&1 | ForEach-Object { "$_" })
  $t = $o -join "`n"
  Check 'the restart banner prints (title, gates, the RESTART rule)' ($t -match 'A RESTART IS NEEDED' -and $t -match 'hashing-1, load-cold' -and $t -match 'type RESTART' -and $t -notmatch 'is not recognized')
  Check 'typing RESTART answers yes' ($script:answer -eq $true)
  Check 'the restart banner is in the runner log' ((Get-Content -LiteralPath (Join-Path $WorkDir 'runner.log') -Raw -Encoding UTF8) -match 'A RESTART IS NEEDED')
} finally { Remove-Item -LiteralPath $WorkDir -Recurse -Force -ErrorAction SilentlyContinue }

if ($fails) { Write-Host "$fails failed"; exit 1 }
Write-Host 'NeedsYou tests passed'
