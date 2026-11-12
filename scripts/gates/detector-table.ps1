<#
.SYNOPSIS
  Collects the live detector check of a night of recorder gates into one table.

.DESCRIPTION
  Every recorder gate (recorder-gate.ps1, kill-gate.ps1) saves detector-check.json in its run
  folder: the shared upstream breaks the harness found, how many the recorder's live detectors
  (repeated_block, zero_gap) marked, the false alarms and lead-in events, and the recorder's
  detector counters (longest time on one block, blocks over budget, detectors switched off).
  This script reads the run folders under -WorkRoot whose name carries a timestamp at or after
  -Since and writes docs\evidence\phase-2b\detector-table.txt. It records and times nothing, so no
  lock or overlay check applies.

    powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\gates\detector-table.ps1 -Since '2026-11-13 00:00'
#>
[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)] [datetime]$Since,
  [string]$WorkRoot = 'D:\STELMAR-Gate',
  [string]$OutFile,
  [string]$Machine = 'Legion'
)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo = (Resolve-Path (Join-Path $here '..\..')).Path
Import-Module (Join-Path $repo 'scripts\lib\PsEnv.psm1') -Force
Set-WbCleanPSModulePath
Set-WbInvariantCulture
Import-Module (Join-Path $repo 'scripts\lib\Redact.psm1') -Force
if (-not $OutFile) { $OutFile = Join-Path $repo 'docs\evidence\phase-2b\detector-table.txt' }
New-Item -ItemType Directory -Force -Path (Split-Path -Parent $OutFile) | Out-Null

$rows = @()
foreach ($d in (Get-ChildItem -LiteralPath $WorkRoot -Directory -ErrorAction SilentlyContinue | Sort-Object Name)) {
  if ($d.Name -notmatch '-(\d{8})-(\d{6})$') { continue }
  $stamp = [datetime]::ParseExact("$($Matches[1])$($Matches[2])", 'yyyyMMddHHmmss', [Globalization.CultureInfo]::InvariantCulture)
  if ($stamp -lt $Since) { continue }
  $f = Join-Path $d.FullName 'detector-check.json'
  if (-not (Test-Path -LiteralPath $f)) { continue }
  $rows += (Get-Content -LiteralPath $f -Raw | ConvertFrom-Json)
}

$rev = (& git -C $repo rev-parse --short HEAD) -join ''
$lines = @()
$lines += 'Live detector table: the recorder''s repeated_block and zero_gap detectors against the shared upstream breaks of each gate run (Phase 2b)'
$lines += "source: commit $rev; machine: $Machine; run folders at or after $($Since.ToString('yyyy-MM-dd HH:mm')) with a detector-check.json (written by the gate scripts)"
$lines += 'rule: a shared upstream break is matched when a repeated_block or zero_gap event lies within +-50 ms of the break''s span in the app''s audio time; an event with no shared break is a false alarm unless it lies before the first source sample that reached the recording (lead-in)'
$lines += ''
$lines += 'run | gate | profile | audio min | shared breaks | matched | missed | before recording | false alarms | lead-in | repeated_block events | zero_gap events | longest detector time on one block (us) | blocks over budget | detectors switched off'
$tot = [pscustomobject]@{ Min = 0.0; Breaks = 0; Matched = 0; Missed = 0; Before = 0; False = 0; Lead = 0; Over = 0; Off = 0; Max = 0 }
foreach ($r in $rows) {
  $c = $r.counters
  $lines += ('{0} | {1} | {2} | {3:n1} | {4} | {5} | {6} | {7} | {8} | {9} | {10} | {11} | {12} | {13} | {14}' -f $r.run, $r.gate, $r.profile, [double]$r.audio_minutes, $r.breaks, $r.matched, $r.missed, $r.before_recording, $r.false_alarms, $r.lead_in, $c.repeated_blocks, $c.zero_gaps, $c.detector_max_block_us, $c.detector_over_budget_blocks, $c.detectors_disabled)
  $tot.Min += [double]$r.audio_minutes; $tot.Breaks += [int]$r.breaks; $tot.Matched += [int]$r.matched; $tot.Missed += [int]$r.missed
  $tot.Before += [int]$r.before_recording; $tot.False += [int]$r.false_alarms; $tot.Lead += [int]$r.lead_in
  $tot.Over += [int]$c.detector_over_budget_blocks; $tot.Off += [int]$c.detectors_disabled
  $tot.Max = [math]::Max([double]$tot.Max, [double]$c.detector_max_block_us)
}
$lines += ('total | {0} runs | | {1:n1} | {2} | {3} | {4} | {5} | {6} | {7} | | | {8} | {9} | {10}' -f $rows.Count, $tot.Min, $tot.Breaks, $tot.Matched, $tot.Missed, $tot.Before, $tot.False, $tot.Lead, $tot.Max, $tot.Over, $tot.Off)
$lines += ''
$lines += $(if ($rows.Count -eq 0) { 'no detector-check.json found: nothing to report' } else { "false alarms outside lead-in: $($tot.False); misses: $($tot.Missed); detectors switched off: $($tot.Off); blocks over the time budget: $($tot.Over)" })
Write-WbEvidence -Path $OutFile -Lines $lines -MachineLabel $Machine
Write-Host "DONE: $OutFile ($($rows.Count) runs)"
