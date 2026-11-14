<#
.SYNOPSIS
  Proves a built gate-pack\ is safe to copy to a laptop.

.DESCRIPTION
  Checks, and exits 1 with the list if any fails:
    1. every file matches PACK-MANIFEST.json and nothing else is in the folder;
    2. no source code, build file, development tool, wb-emulate, private audio or secret-looking string
       (WAV files must be on the manifest's public-clip allow-list);
    3. every gate script the registry names parses in Windows PowerShell 5.1 and is ASCII only;
    4. the release app does not contain the gate hooks (WB_GATE_ strings);
    5. static network check: no program in the pack imports Winsock, WinHTTP or WinINet (the
       positive control, netstat.exe, must be found as a network program or the check says it is blind);
    6. the registry is valid and every ledger entry matches exactly one row of docs\GATES.md.
  It also lists the pack's engines whose imports are not readable. Run after build-gate-pack.ps1.
#>
[CmdletBinding()]
param([string]$Pack = '')
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo = (Resolve-Path (Join-Path $here '..')).Path
if (-not $Pack) { $Pack = Join-Path $repo 'gate-pack' }
$Pack = (Resolve-Path $Pack).Path
Import-Module (Join-Path $repo 'scripts\lib\PackBuild.psm1') -Force
Import-Module (Join-Path $repo 'scripts\lib\PackCore.psm1') -Force
Import-Module (Join-Path $repo 'scripts\lib\PeImports.psm1') -Force

$problems = New-Object System.Collections.Generic.List[string]
$ok = New-Object System.Collections.Generic.List[string]

# 1. manifest
$mp = @(Test-PackManifest -Root $Pack)
if ($mp.Count -gt 0) { foreach ($m in $mp) { $problems.Add("manifest: $m") } } else { $ok.Add('every file matches PACK-MANIFEST.json and nothing else is in the folder') }
$pm = Get-Content -LiteralPath (Join-Path $Pack 'PACK-MANIFEST.json') -Raw -Encoding UTF8 | ConvertFrom-Json

# 2. forbidden content
$allowed = @($pm.allowed_wav)
$fb = @(Get-PackForbidden -Root $Pack -AllowedWav $allowed)
if ($fb.Count -gt 0) { foreach ($f in $fb) { $problems.Add("$($f.Kind): $($f.Path) ($($f.Why))") } } else { $ok.Add('no source, development tool, wb-emulate, private audio or secret-looking string') }

# 3. scripts parse and are ASCII
$bad = 0
foreach ($f in @(Get-ChildItem -LiteralPath $Pack -Recurse -File | Where-Object { $_.Extension -in '.ps1', '.psm1' -and $_.FullName -notlike (Join-Path $Pack 'work\*') })) {
  $latin = [IO.File]::ReadAllText($f.FullName, [Text.Encoding]::GetEncoding(28591))
  if ([regex]::IsMatch($latin, '[^\x00-\x7F]')) { $problems.Add("not ASCII: $($f.FullName.Substring($Pack.Length + 1))"); $bad++ }
  $errs = $null
  [void][System.Management.Automation.Language.Parser]::ParseFile($f.FullName, [ref]$null, [ref]$errs)
  if ($errs -and @($errs).Count -gt 0) { $problems.Add("does not parse: $($f.FullName.Substring($Pack.Length + 1)): $($errs[0].Message)"); $bad++ }
}
if ($bad -eq 0) { $ok.Add('every script parses and is ASCII only') }

# 4. the release app has no gate hooks
$rel = Join-Path $Pack 'target\release\stelmar-voice-workbench.exe'
if (Test-Path -LiteralPath $rel) {
  if (Test-PackExeStrings -Path $rel -Text 'WB_GATE_') { $problems.Add('the release app contains WB_GATE_ strings: it is a gate build') } else { $ok.Add('the release app has no WB_GATE_ strings') }
  $gate = Join-Path $Pack 'target\gate\stelmar-voice-workbench.exe'
  if (-not (Test-Path -LiteralPath $gate)) { $problems.Add('the gate-build app is missing') }
  elseif (-not (Test-PackExeStrings -Path $gate -Text 'WB_GATE_')) { $problems.Add('the gate app has no WB_GATE_ strings: it is not a gate build') }
  else { $ok.Add('the gate app has the gate hooks (so the gates can run)') }
} else { $problems.Add('the release app is missing') }

# 5. static network imports
$exes = @(Get-ChildItem -LiteralPath $Pack -Recurse -File | Where-Object { $_.Extension -in '.exe', '.dll' } | ForEach-Object { $_.FullName })
$net = Test-PackNetworkImports -Files $exes
if (-not $net.ControlOk) { $problems.Add('the static network check is blind: the positive control (netstat.exe) was not found as a network program') }
foreach ($f in @($net.Findings)) { $problems.Add("imports a network library: $($f.Path.Substring($Pack.Length + 1)) ($(@($f.Network) -join ', '))") }
if ($net.ControlOk -and @($net.Findings).Count -eq 0) { $ok.Add(('no program in the pack imports Winsock, WinHTTP or WinINet ({0} programs read; control {1} found)' -f ($exes.Count - @($net.Unreadable).Count), (Split-Path -Leaf $net.Control.Path))) }
foreach ($u in @($net.Unreadable)) { Write-Host ('note: imports not readable: ' + $u.Path.Substring($Pack.Length + 1) + ' (' + $u.Error + ')') }

# 6. the registry
try {
  $manifest = Read-PackManifest -Path (Join-Path $Pack 'scripts\gate-pack\gates.manifest.json')
  $lm = @(Test-PackLedgerMatches -Manifest $manifest -GatesMdPath (Join-Path $repo 'docs\GATES.md') | Where-Object { $_.Count -ne 1 })
  if ($lm.Count -gt 0) { foreach ($l in $lm) { $problems.Add("ledger: $($l.Gate): '$($l.Match)' matches $($l.Count) rows of docs\GATES.md") } } else { $ok.Add('the registry is valid and every ledger entry matches exactly one row of docs\GATES.md') }
} catch { $problems.Add("registry: $($_.Exception.Message)") }

Write-Host ''
foreach ($l in $ok) { Write-Host "  ok   $l" -ForegroundColor Green }
foreach ($l in $problems) { Write-Host "  FAIL $l" -ForegroundColor Red }
Write-Host ''
if ($problems.Count -gt 0) { Write-Host "check-gate-pack: $($problems.Count) problem(s)" -ForegroundColor Red; exit 1 }
Write-Host 'check-gate-pack: the pack is clean' -ForegroundColor Green
exit 0
