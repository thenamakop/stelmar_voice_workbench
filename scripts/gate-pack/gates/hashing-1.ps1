# Gate hashing-1: the first run of R5 (the app hashes the model files in the background while it records),
# the one that follows the real restart, so the file cache is cold. Wraps scripts\gates\recorder-gate.ps1
# -Gate hashing, which checks that the models and the recording are on the same disk and that at least
# 1 GB was hashed inside the recording. This wrapper adds the raw read speed of the disk, the warm-cache
# reference and the ratio, and refuses a run that did not follow a restart (it is repeated after one).
# Windows PowerShell 5.1, ASCII only.
param([string]$CtxFile, [string]$ResultFile)
$ErrorActionPreference = 'Stop'
$lib = Join-Path $PSScriptRoot '..\..\lib'
Import-Module (Join-Path $lib 'PackCore.psm1') -Force
Import-Module (Join-Path $lib 'PackGate.psm1') -Force
$ctx = Read-PackCtx -CtxFile $CtxFile
Initialize-PackGate -Ctx $ctx
Import-Module (Join-Path $lib 'RecorderGates.psm1') -Force
Import-Module (Join-Path $lib 'Hashing.psm1') -Force
$paths = Get-PackPaths -Ctx $ctx

Invoke-PackGateBody -Ctx $ctx -Id 'hashing-1' -ResultFile $ResultFile -Body {
  $quick = [bool]$ctx.quick
  $extra = New-Object System.Collections.Generic.List[string]
  $large = Join-Path $paths.Models 'ggml-large-v3-q5_0.bin'
  if (-not (Test-Path -LiteralPath $large)) { $large = @(Get-ChildItem -LiteralPath $paths.Models -Filter 'ggml-*.bin' | Sort-Object Length -Descending | Select-Object -First 1)[0].FullName }
  # The disk's own speed, read before the app starts: a raw read of 128 MB from the tail of a model the
  # app has not been asked to read yet.
  $raw = $null
  try { $raw = Measure-PackReadRate -Path $large -Bytes 128MB } catch { $raw = $null }
  $extra.Add(('  raw read of 128 MB from the end of {0} before the app started: {1}' -f (Split-Path $large -Leaf), $(if ($null -ne $raw) { ('{0:n0} MB/s (the disk''s own speed with a cold cache; 128 MB of this model is now warm)' -f $raw) } else { 'not measured' })))
  $r = Invoke-PackRecorderGate -Ctx $ctx -Id 'hashing-1' -Gate 'hashing' -FullMinutes 10 -QuickMinutes 4 -Threshold 'the app hashed at least 1 GB of model files in background mode while recording; models and recording on the same disk; the run followed a restart; recording equals both references'
  $evPath = @($r['evidence'] | Where-Object { $_ -match 'hashing-.*\.txt$' } | Select-Object -First 1)
  $ev = @()
  if ($evPath.Count -gt 0 -and (Test-Path -LiteralPath $evPath[0])) { $ev = @(Get-Content -LiteralPath $evPath[0] -Encoding UTF8) }
  $rows = @(ConvertFrom-PackHashEvidence -Lines $ev)
  $rate = Get-PackHashRate -Rows $rows
  $afterRestart = (@($ev | Where-Object { $_ -match 'started after a reboot: yes' }).Count -gt 0)
  $extra.Add(('  the app hashed {0} files at {1} MB/s while recording' -f $rows.Count, $(if ($null -ne $rate) { ('{0:n0}' -f $rate) } else { 'unknown' })))
  $extra.Add(('  started after a restart: {0} (the run counts as cold only then)' -f $(if ($afterRestart) { 'yes' } else { 'NO' })))
  $note = Join-Path ([string]$ctx.gate_dir) 'hashing-1-additions.txt'
  Set-Content -LiteralPath $note -Value @($extra) -Encoding UTF8
  $r['evidence'] = @($r['evidence']) + @($note)
  if ($null -eq $r['measured']) { $r['measured'] = [ordered]@{} }
  $r['measured']['hash_mb_per_s'] = $rate
  $r['measured']['raw_read_mb_per_s'] = $raw
  $r['measured']['after_restart'] = $afterRestart
  $r['measured']['hashed_files'] = $rows.Count
  if ($r['status'] -eq 'PASS' -and -not $afterRestart) {
    $r['status'] = 'INVALID'
    $r['headline'] = 'the run did not follow a restart, so the file cache was probably warm; run it again straight after a restart'
  }
  $r
}
