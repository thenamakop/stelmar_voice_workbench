# Gate rename-engines: with every engine renamed away the app still records and says so on the Record
# screen; with the engines present it does not. Two 3-minute recordings on the audio interface (soak, the
# full recorder checks) plus two looks at the Record screen through the window's accessibility tree
# (needs the interactive desktop). Wraps scripts\gates\rename-engines.ps1. Windows PowerShell 5.1, ASCII only.
param([string]$CtxFile, [string]$ResultFile)
$ErrorActionPreference = 'Stop'
$lib = Join-Path $PSScriptRoot '..\..\lib'
Import-Module (Join-Path $lib 'PackCore.psm1') -Force
Import-Module (Join-Path $lib 'PackGate.psm1') -Force
$ctx = Read-PackCtx -CtxFile $CtxFile
Initialize-PackGate -Ctx $ctx
$paths = Get-PackPaths -Ctx $ctx

Invoke-PackGateBody -Ctx $ctx -Id 'rename-engines' -ResultFile $ResultFile -Body {
  $quick = [bool]$ctx.quick
  $thr = 'both 3-minute recordings pass their own checks; the banner "No speech engine found." shows with the engines renamed away and not with them present'
  $gdir = [string]$ctx.gate_dir
  $scratch = Join-Path $gdir 'scratch'
  $a = @(Get-PackLegacyCommon -Ctx $ctx) + @('-RigPrefer', [string]$ctx.rig_prefer, '-SecondReference', '-Minutes', '3', '-Scratch', (ConvertTo-PackArg $scratch))
  $e = Invoke-PackLegacyGate -Ctx $ctx -Script 'rename-engines.ps1' -Arguments $a -EvidenceName ("rename-engines-$([string]$ctx.profile).txt") -TimeoutMinutes 40
  if ($e.TimedOut) { return (New-PackResult -Id 'rename-engines' -Status 'INVALID' -Headline 'the renamed-engines gate did not finish in time' -Evidence @($e.Log) -Quick $quick) }
  $ev = @()
  if ($e.Evidence -and (Test-Path -LiteralPath $e.Evidence)) { $ev = @(Get-Content -LiteralPath $e.Evidence -Encoding UTF8) }
  $verdict = @($ev | Where-Object { $_ -match '^verdict:' } | Select-Object -First 1)
  $reason = @($ev | Where-Object { $_ -match '(RIG REFUSED|RIG SELF-TEST|Overlay tools|Only [0-9.]+ GB|An engine build)' } | Select-Object -First 1)
  if ($e.ExitCode -ne 0 -and $reason.Count -gt 0) {
    $msg = [string]$reason[0]
    if ($msg -match '(RIG REFUSED.*|RIG SELF-TEST.*|Overlay tools.*|Only [0-9.]+ GB.*|An engine build.*)$') { $msg = $Matches[1] }
    return (New-PackResult -Id 'rename-engines' -Status (Get-PackErrorStatus $msg) -Headline $msg -Evidence @($e.Evidence, $e.Log) -Quick $quick)
  }
  $status = 'INVALID'
  if ($e.ExitCode -eq 0 -and $verdict.Count -gt 0 -and $verdict[0] -match 'PASS') { $status = 'PASS' }
  elseif ($e.ExitCode -eq 1 -and $verdict.Count -gt 0 -and $verdict[0] -match 'FAIL') { $status = 'FAIL' }
  $banner = @($ev | Where-Object { $_ -match "found in the window: (True|False)" })
  $head = $(if ($verdict.Count -gt 0) { ([string]$verdict[0]) -replace '^verdict:\s*', '' } else { 'no verdict line in the evidence' })
  New-PackResult -Id 'rename-engines' -Status $status -Headline $head -Measured ([ordered]@{ exit_code = $e.ExitCode; banner_lines = @($banner | ForEach-Object { $_.Trim() }) }) -Threshold $thr -Evidence @(@($e.Evidence, $e.Log) | Where-Object { $_ }) -Quick $quick
}
