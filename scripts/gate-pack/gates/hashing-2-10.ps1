# Gate hashing-2-10: runs 2 to 10 of R5 (the app hashes the model files in the background while it records):
# four purged runs (Windows' standby file cache emptied first, administrator needed) and five warm runs.
# A purged run counts as cold only if the app read the models at most 0.6 times as fast as the warm runs
# did on this laptop; if fewer than four count, up to two more purged runs are made. Wraps
# scripts\gates\recorder-gate.ps1 -Gate hashing. Windows PowerShell 5.1, ASCII only.
param([string]$CtxFile, [string]$ResultFile)
$ErrorActionPreference = 'Stop'
$lib = Join-Path $PSScriptRoot '..\..\lib'
Import-Module (Join-Path $lib 'PackCore.psm1') -Force
Import-Module (Join-Path $lib 'PackGate.psm1') -Force
$ctx = Read-PackCtx -CtxFile $CtxFile
Initialize-PackGate -Ctx $ctx
Import-Module (Join-Path $lib 'RecorderGates.psm1') -Force
Import-Module (Join-Path $lib 'Hashing.psm1') -Force

Invoke-PackGateBody -Ctx $ctx -Id 'hashing-2-10' -ResultFile $ResultFile -Body {
  $quick = [bool]$ctx.quick
  $thr = 'every run: zero overruns and no gap while the models are hashed in background mode (at least 1 GB, 10 s inside the recording); recording equals both references; a purged run counts as cold only at most 0.6 times the warm-cache rate of this laptop'
  $plan = New-Object System.Collections.Generic.List[object]
  if ($quick) { $plan.Add(@{ Tag = 'run2'; Kind = 'purged' }); $plan.Add(@{ Tag = 'run3'; Kind = 'warm' }) }
  else { foreach ($i in 2..5) { $plan.Add(@{ Tag = "run$i"; Kind = 'purged' }) }; foreach ($i in 6..10) { $plan.Add(@{ Tag = "run$i"; Kind = 'warm' }) } }
  $needCold = $(if ($quick) { 1 } else { 4 })
  $runs = New-Object System.Collections.Generic.List[object]
  $evidence = New-Object System.Collections.Generic.List[string]
  $notes = New-Object System.Collections.Generic.List[string]
  $rig = ''
  $rigKind = ''
  $doRun = {
    param($step)
    $purged = $false
    if ($step.Kind -eq 'purged') { $purged = [bool](Clear-PackStandbyList) }
    $r = Invoke-PackRecorderGate -Ctx $ctx -Id 'hashing-2-10' -Gate 'hashing' -FullMinutes 10 -QuickMinutes 4 -RunTag $step.Tag -Threshold $thr
    foreach ($e in @($r['evidence'])) { if ($e -and -not $evidence.Contains([string]$e)) { $evidence.Add([string]$e) } }
    $rate = $null
    $evFile = @($r['evidence'] | Where-Object { $_ -match ('hashing-[^\\]*-' + [regex]::Escape($step.Tag) + '\.txt$') } | Select-Object -First 1)
    if ($evFile.Count -gt 0 -and (Test-Path -LiteralPath $evFile[0])) { $rate = Get-PackHashRate -Rows @(ConvertFrom-PackHashEvidence -Lines @(Get-Content -LiteralPath $evFile[0] -Encoding UTF8)) }
    [pscustomobject]@{ Tag = $step.Tag; Kind = $step.Kind; Purged = $purged; Status = [string]$r['status']; Rate = $rate; Shared = [int]$(if ($r['measured']) { $r['measured']['shared_breaks'] } else { 0 }); AppOnly = [int]$(if ($r['measured']) { $r['measured']['app_only_breaks'] } else { 0 }); Result = $r }
  }
  foreach ($step in $plan) {
    $one = & $doRun $step
    if ($one.Status -eq 'PENDING') { return $one.Result }
    if ($null -eq $one.Result['measured']) { return $one.Result }
    $runs.Add($one)
    if ($one.Result['rig']) { $rig = [string]$one.Result['rig'] }
    if ($one.Result['measured'] -and $one.Result['measured']['rig']) { $rigKind = [string]$one.Result['measured']['rig'] }
    if ($one.Status -eq 'FAIL') { break }
  }
  $v = Get-PackHashingVerdict -Runs @($runs.ToArray()) -NeedCold $needCold
  # Fewer cold runs than needed: up to two more purged runs, each judged against the warm rate.
  $extra = 0
  while ($v.Status -eq 'INFO' -and $v.ColdCount -lt $needCold -and $extra -lt 2 -and $v.WarmRate) {
    $extra++
    $one = & $doRun @{ Tag = ("extra$extra"); Kind = 'purged' }
    if ($one.Status -eq 'PENDING') { break }
    $runs.Add($one)
    $v = Get-PackHashingVerdict -Runs @($runs.ToArray()) -NeedCold $needCold
  }
  $sum = Join-Path ([string]$ctx.gate_dir) 'hashing-2-10-summary.txt'
  $lines = New-Object System.Collections.Generic.List[string]
  $lines.Add('hashing runs 2 to 10 (purged = the standby file cache was emptied before the run; counts as cold only at most 0.6 of the warm rate)')
  $lines.Add(('  warm-cache rate of this laptop (median of the warm runs): {0}' -f $(if ($v.WarmRate) { ('{0:n0} MB/s' -f $v.WarmRate) } else { 'not measured' })))
  $lines.Add('  run | kind | purge done | recorder verdict | app hash MB/s | counts as cold | shared breaks | app-only breaks | note')
  foreach ($row in $v.Rows) {
    $src = @($runs | Where-Object { $_.Tag -eq $row.Tag })[0]
    $lines.Add(('  {0} | {1} | {2} | {3} | {4} | {5} | {6} | {7} | {8}' -f $row.Tag, $row.Kind, $(if ($row.Kind -eq 'purged') { $src.Purged } else { 'n/a' }), $row.Status, $(if ($null -ne $row.Rate) { ('{0:n0}' -f $row.Rate) } else { 'n/a' }), $(if ($row.Kind -eq 'purged') { $row.CountsAsCold } else { 'n/a' }), $row.Shared, $row.AppOnly, $row.Note))
  }
  $lines.Add('  caveat: a rate above 0.6 of the warm rate cannot be shown to be cold, so that run is not counted as cold; a rate below it is consistent with a cold cache but is not proof, because SHA-256 can itself be slower than the disk')
  $coldRows = @($v.Rows | Where-Object { $_.Kind -eq 'purged' })
  $sharedCold = ($coldRows | Measure-Object -Property Shared -Sum).Sum
  $warmRowsAll = @($v.Rows | Where-Object { $_.Kind -eq 'warm' })
  $sharedWarm = ($warmRowsAll | Measure-Object -Property Shared -Sum).Sum
  $lines.Add(('  shared upstream breaks: {0} in the purged runs, {1} in the warm runs' -f [int]$sharedCold, [int]$sharedWarm))
  Set-Content -LiteralPath $sum -Value @($lines) -Encoding UTF8
  $evidence.Add($sum)
  foreach ($l in @($v.Rows | Where-Object { $_.Kind -eq 'purged' -and -not $_.CountsAsCold } | ForEach-Object { "$($_.Tag): $($_.Note)" })) { $notes.Add($l) }
  $meas = [ordered]@{ runs = $runs.Count; cold_runs = $v.ColdCount; warm_rate_mb_s = $v.WarmRate; shared_breaks_purged = [int]$sharedCold; shared_breaks_warm = [int]$sharedWarm; extra_purged_runs = $extra; rig = $rigKind }
  New-PackResult -Id 'hashing-2-10' -Status $v.Status -Headline $v.Headline -Measured $meas -Threshold $thr -Rig $rig -Evidence @($evidence.ToArray()) -Notes @($notes.ToArray()) -Quick $quick -SharedBreaks ([int]$sharedCold + [int]$sharedWarm)
}
