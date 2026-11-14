<#
  Ingest.psm1: turns the results zip of a gate-pack run into docs/GATES.md changes. Development side only.

  The pure parts (tested in scripts/tests/Ingest.Tests.ps1):
    Get-IngestRequiredRoles   which laptops a ledger row needs (low, mid, or both) from its machine cell
    Read-IngestAnnotations    the gate-pack results already written into a Status cell
    Get-IngestStatusCell      the new Status cell: the old status kept, the gate-pack results added
    Update-IngestGatesLines   applies one run's results to the lines of docs/GATES.md
    ConvertTo-IngestRedacted  removes user paths and secret values from text copied into docs/evidence

  Rules (docs/prompts/laptop-gate-pack.md):
    - PROVISIONAL and DEFERRED rows become PASS or FAIL only when every laptop the row needs has a verdict;
      the old status stays in the cell as "[was: ...]".
    - A FAIL adds a "Phase N REOPENED" row to the Open items and is never overwritten by a later PASS from
      the other laptop.
    - INFO results never close anything: a row that was DEFERRED or OPEN becomes "RESULT IN, owner to read".
    - Open items are never closed by ingest, only annotated.
    - A run on the VB-CABLE fallback rig keeps the rows that need the hardware audio interface open.
    - An emulated run (dry run) never touches the real rows: it goes to its own block "Gate pack runs".

  Windows PowerShell 5.1, ASCII only.
#>

Set-StrictMode -Version 2

Import-Module (Join-Path $PSScriptRoot 'PackNative.psm1')
$script:Marker = ' ;; gate pack: '

function ConvertTo-IngestRedacted {
  [CmdletBinding()]
  param([AllowEmptyString()] [string[]]$Line)
  foreach ($l in @($Line)) {
    $s = $l
    $s = $s -replace '[A-Za-z]:\\Users\\[^\\"''<>|]+', '%USERPROFILE%'
    $s = $s -replace '\b([A-Za-z0-9_]*(?:KEY|TOKEN|SECRET|PASSWORD))=\S*', '$1=<redacted>'
    $s
  }
}

# Pure. 'low-baseline laptop' -> low; 'mid laptop' -> mid; both named, or neither (a Legion row that the
# laptops re-run) -> both.
function Get-IngestRequiredRoles {
  [CmdletBinding()]
  param([string]$MachineCell)
  $low = $MachineCell -match '(?i)low-baseline|low laptop'
  $mid = $MachineCell -match '(?i)\bmid\b'
  if ($low -and -not $mid) { return @('low') }
  if ($mid -and -not $low) { return @('mid') }
  @('low', 'mid')
}

# Pure. Splits a Status cell into the base (status text with its "[was: ...]") and the gate-pack results
# already recorded: role -> @{ State; Machine; Date }.
function Read-IngestAnnotations {
  [CmdletBinding()]
  param([string]$Cell)
  $i = $Cell.IndexOf($script:Marker, [StringComparison]::Ordinal)
  $base = $Cell
  $tail = ''
  if ($i -ge 0) { $base = $Cell.Substring(0, $i); $tail = $Cell.Substring($i + $script:Marker.Length) }
  $res = [ordered]@{}
  foreach ($m in [regex]::Matches($tail, '(low|mid)/([a-z0-9-]+) (PASS|FAIL|INFO|FALLBACK) \(([^,)]*), ([^)]*)\)')) {
    $res[$m.Groups[1].Value + '/' + $m.Groups[2].Value] = @{ State = $m.Groups[3].Value; Machine = $m.Groups[4].Value; Date = $m.Groups[5].Value }
  }
  [pscustomobject]@{ Base = $base.Trim(); Results = $res }
}

# Pure. -Results: "role/gate" -> @{State; Machine; Date}. -Required: the "role/gate" pairs the row needs
# (every laptop it names times every gate that feeds it). -Kind: verdict or info. -Table: gates or open.
function Get-IngestStatusCell {
  [CmdletBinding()]
  param([string]$Base, $Results, [string[]]$Required, [string]$Kind, [string]$Table)
  $was = $Base
  if ($Base -match '^(.*) \[was: (.*)\]$') { $was = $Matches[2] }
  $tailParts = @($Results.Keys | ForEach-Object { '{0} {1} ({2}, {3})' -f $_, $Results[$_].State, $Results[$_].Machine, $Results[$_].Date })
  $tail = $script:Marker + ($tailParts -join '; ')
  $states = @($Results.Values | ForEach-Object { $_.State })
  $have = @($Results.Keys)
  $missing = @($Required | Where-Object { $have -notcontains $_ })
  $head = $was
  if ($Kind -eq 'info') {
    if ($was -match '^(?i)(OPEN|DEFERRED)') { $head = 'RESULT IN, owner to read [was: ' + $was + ']' }
  } elseif ($Table -eq 'open') {
    if ($states -contains 'FAIL') { $head = 'OPEN (gate pack FAIL, see the REOPENED row) [was: ' + $was + ']' }
    elseif (($states -contains 'FALLBACK') -or $missing.Count -gt 0) { $head = $was }
    else { $head = 'RESULT IN: PASS everywhere it is needed, owner to close [was: ' + $was + ']' }
  } else {
    if ($states -contains 'FAIL') { $head = 'FAIL (gate pack) [was: ' + $was + ']' }
    elseif ($states -contains 'FALLBACK') { $head = $was }
    elseif ($missing.Count -eq 0 -and $states.Count -gt 0) { $head = 'PASS (gate pack) [was: ' + $was + ']' }
  }
  if ($missing.Count -gt 0 -and $states.Count -gt 0 -and $Kind -ne 'info') { $tail += '; still to run: ' + ($missing -join ', ') }
  ($head + $tail)
}

# Splits one table line into its cells (a literal pipe in a cell is written \|).
function Split-IngestRow {
  param([string]$Line)
  $inner = $Line.Trim().Trim('|')
  @([regex]::Split($inner, '(?<!\\)\|') | ForEach-Object { $_.Trim() })
}

function Join-IngestRow {
  param([string[]]$Cells)
  '| ' + (($Cells | ForEach-Object { $_ -replace '(?<!\\)\|', '\|' }) -join ' | ') + ' |'
}

# The table (gates, open or $null) each line of the file belongs to.
function Get-IngestTableMap {
  param([AllowEmptyString()] [string[]]$Lines)
  $map = New-Object 'string[]' $Lines.Count
  $t = $null
  for ($i = 0; $i -lt $Lines.Count; $i++) {
    if ($Lines[$i] -match '^##\s+Gates\s*$') { $t = 'gates' } elseif ($Lines[$i] -match '^##\s+Open items\s*$') { $t = 'open' } elseif ($Lines[$i] -match '^##\s+') { $t = $null }
    $map[$i] = $t
  }
  , $map
}

# Applies one run to the lines of GATES.md. -Summary: the run's summary.json (gates with their ledger
# entries, pending and not_in_pack). Returns Lines, Changes (text for the preview), Reopened (ids),
# Waiting (ids that gave no verdict).
function Update-IngestGatesLines {
  [CmdletBinding()]
  param(
    [Parameter(Mandatory = $true)] [AllowEmptyString()] [string[]]$Lines,
    [Parameter(Mandatory = $true)] $Summary,
    [Parameter(Mandatory = $true)] [string]$Role,
    [string]$Date,
    [string]$EvidenceDir = ''
  )
  $out = New-Object System.Collections.Generic.List[string]
  foreach ($l in $Lines) { $out.Add($l) }
  $changes = New-Object System.Collections.Generic.List[string]
  $reopened = New-Object System.Collections.Generic.List[string]
  $waiting = New-Object System.Collections.Generic.List[string]
  $machine = [string]$Summary.machine
  $created = [string]$Summary.created
  $map = Get-IngestTableMap -Lines @($out.ToArray())
  # Every gate that feeds a ledger row, whatever its state in this run: a row closes only when all of them
  # have a verdict on every laptop the row needs.
  $feeders = @{}
  foreach ($set in @($Summary.gates), @($Summary.pending), @($Summary.not_in_pack)) {
    foreach ($e in @($set)) {
      if ($null -eq $e -or -not $e.PSObject.Properties['ledger']) { continue }
      foreach ($le in @($e.ledger)) {
        $k = [string]$le.table + '|' + [string]$le.match
        if (-not $feeders.ContainsKey($k)) { $feeders[$k] = New-Object System.Collections.Generic.List[string] }
        if (-not $feeders[$k].Contains([string]$e.id)) { $feeders[$k].Add([string]$e.id) }
      }
    }
  }
  foreach ($g in @($Summary.gates)) {
    $st = [string]$g.status
    $rig = ''
    $fallback = $false
    if ($g.PSObject.Properties['measured'] -and $g.measured -and $g.measured.PSObject.Properties['rig']) { $rig = [string]$g.measured.rig }
    if ($rig -eq 'vb-cable-fallback' -or ([string]$g.rig -match '(?i)fallback')) { $fallback = $true }
    if (@('PASS', 'FAIL', 'INFO') -notcontains $st) { $waiting.Add("$($g.id) ($st)"); continue }
    foreach ($le in @($g.ledger)) {
      $tbl = [string]$le.table
      $needsHw = ($le.PSObject.Properties['needs_hardware_interface'] -and [bool]$le.needs_hardware_interface)
      $kind = [string]$le.kind
      $thisState = $st
      if ($needsHw -and $fallback -and $st -ne 'FAIL') { $thisState = 'FALLBACK' }
      elseif ($kind -eq 'info' -and $st -eq 'PASS') { $thisState = 'INFO' }
      $cellIx = $(if ($tbl -eq 'gates') { 1 } else { 2 })
      $hit = @()
      for ($i = 0; $i -lt $out.Count; $i++) {
        if ($map[$i] -ne $tbl -or $out[$i] -notmatch '^\|' -or $out[$i] -match '^\|\s*(-+\s*\|\s*)+$') { continue }
        $cells = @(Split-IngestRow $out[$i])
        if ($cells.Count -lt 6) { continue }
        if ($cells[$cellIx].IndexOf([string]$le.match, [StringComparison]::Ordinal) -ge 0) { $hit += $i }
      }
      if ($hit.Count -ne 1) { $changes.Add("  ! ledger entry '$($le.match)' of $($g.id) matches $($hit.Count) rows: not updated"); continue }
      $ix = $hit[0]
      $cells = @(Split-IngestRow $out[$ix])
      $ann = Read-IngestAnnotations -Cell $cells[3]
      $ann.Results["$Role/$($g.id)"] = @{ State = $thisState; Machine = $machine; Date = $Date }
      $required = New-Object System.Collections.Generic.List[string]
      $fk = $tbl + '|' + [string]$le.match
      $gateIds = $(if ($feeders.ContainsKey($fk)) { @($feeders[$fk].ToArray()) } else { @([string]$g.id) })
      foreach ($r in @(Get-IngestRequiredRoles -MachineCell $cells[2])) { foreach ($gid in $gateIds) { $required.Add("$r/$gid") } }
      $new = Get-IngestStatusCell -Base $ann.Base -Results $ann.Results -Required @($required.ToArray()) -Kind $kind -Table $tbl
      $new = $new -replace '\|', '/'
      if ($cells[3] -ne $new) {
        $old = $cells[3]
        $cells[3] = $new
        if ($EvidenceDir -and $cells[4].IndexOf($EvidenceDir, [StringComparison]::Ordinal) -lt 0) { $cells[4] = $cells[4] + '; gate pack: `' + $EvidenceDir + '`' }
        $out[$ix] = Join-IngestRow $cells
        $changes.Add(("  line {0}: {1} [{2}] {3}" -f ($ix + 1), $g.id, $Role, $st))
        $changes.Add('    was: ' + $(if ($old.Length -gt 160) { $old.Substring(0, 160) + ' ...' } else { $old }))
        $changes.Add('    now: ' + $(if ($new.Length -gt 160) { $new.Substring(0, 160) + ' ...' } else { $new }))
      }
      if ($st -eq 'FAIL' -and $kind -eq 'verdict') {
        $phase = [string]$cells[0]
        $tag = "[ingest $created $($g.id)]"
        $already = @($out | Where-Object { $_.IndexOf($tag, [StringComparison]::Ordinal) -ge 0 -and $_.IndexOf('REOPENED', [StringComparison]::Ordinal) -ge 0 }).Count
        if ($already -eq 0) {
          $row = Join-IngestRow @($phase, 'gate pack', ("Phase $phase REOPENED: gate $($g.id) FAILED on $machine ($Role laptop, $Date): " + ([string]$g.headline -replace '\|', '/') + " $tag"), 'OPEN (reopened by the gate pack)', ('`' + $EvidenceDir + '`'), $Date)
          $last = -1
          for ($i = 0; $i -lt $out.Count; $i++) { if ($map[$i] -eq 'open' -and $out[$i] -match '^\|') { $last = $i } }
          if ($last -ge 0) {
            $out.Insert($last + 1, $row)
            $map = Get-IngestTableMap -Lines @($out.ToArray())
            $reopened.Add([string]$g.id)
            $changes.Add("  new Open items row: Phase $phase REOPENED: $($g.id)")
          }
        }
      }
    }
  }
  [pscustomobject]@{ Lines = @($out.ToArray()); Changes = @($changes.ToArray()); Reopened = @($reopened.ToArray()); Waiting = @($waiting.ToArray()) }
}

# Appends or replaces this run's rows in the "Gate pack runs" block (dry runs and real runs alike).
function Update-IngestRunsBlock {
  [CmdletBinding()]
  param([Parameter(Mandatory = $true)] [AllowEmptyString()] [string[]]$Lines, [Parameter(Mandatory = $true)] $Summary, [string]$Role, [string]$Date, [string]$EvidenceDir)
  $runId = [string]$Summary.created
  $emu = [string]$Summary.emulated
  $kept = New-Object System.Collections.Generic.List[string]
  foreach ($l in $Lines) { if ($l.IndexOf("[run $runId]", [StringComparison]::Ordinal) -lt 0) { $kept.Add($l) } }
  $hasBlock = @($kept | Where-Object { $_ -match '^##\s+Gate pack runs\s*$' }).Count -gt 0
  if (-not $hasBlock) {
    $kept.Add('')
    $kept.Add('## Gate pack runs')
    $kept.Add('')
    $kept.Add('One row per gate of each run of `gate-pack/run-gates.ps1`, written by `scripts/ingest-gate-results.ps1`. A row marked "emulated" is a dry run on the Legion (`emu-low` or `emu-mid`, `-Quick`): it proves the pack and the ingest, and is never evidence for the laptops. Real runs also update the rows of the two tables above.')
    $kept.Add('')
    $kept.Add('| Run | Gate | Result | Machine or profile | Notes | Evidence | Date |')
    $kept.Add('| --- | --- | --- | --- | --- | --- | --- |')
  }
  $who = $(if ($emu) { "emulated: $emu on the Legion (not a laptop)" } else { "$Role laptop, $($Summary.machine)" })
  foreach ($g in @($Summary.gates)) {
    $notes = New-Object System.Collections.Generic.List[string]
    if ([bool]$Summary.quick -or [bool]$g.quick) { $notes.Add('-Quick: not at full length') }
    if ([string]$g.rig) { $notes.Add('rig: ' + [string]$g.rig) }
    $head = ([string]$g.headline -replace '\|', '/')
    if ($head.Length -gt 220) { $head = $head.Substring(0, 220) + ' ...' }
    $notes.Add($head)
    $kept.Add((Join-IngestRow @("run $($Summary.machine) $runId [run $runId]", [string]$g.id, [string]$g.status, $who, ($notes -join '; '), ('`' + $EvidenceDir + '`'), $Date)))
  }
  foreach ($p in @($Summary.pending)) { $kept.Add((Join-IngestRow @("run $($Summary.machine) $runId [run $runId]", [string]$p.id, 'WAITING', $who, ([string]$p.reason -replace '\|', '/'), ('`' + $EvidenceDir + '`'), $Date))) }
  foreach ($p in @($Summary.not_in_pack)) { $kept.Add((Join-IngestRow @("run $($Summary.machine) $runId [run $runId]", [string]$p.id, 'NOT IN THIS PACK', $who, 'the gate was not built into this pack', ('`' + $EvidenceDir + '`'), $Date))) }
  @($kept.ToArray())
}

# Fills the Machines row of a laptop that still says "(fill in on arrival)".
function Update-IngestMachines {
  [CmdletBinding()]
  param([Parameter(Mandatory = $true)] [AllowEmptyString()] [string[]]$Lines, [Parameter(Mandatory = $true)] $Summary, [string]$Role, [string]$Date)
  $fp = $Summary.fingerprint
  if ($null -eq $fp) { return [pscustomobject]@{ Lines = $Lines; Changed = $false } }
  $name = $(if ($Role -eq 'low') { 'Low-baseline laptop' } else { 'Mid laptop' })
  $out = @($Lines)
  $changed = $false
  for ($i = 0; $i -lt $out.Count; $i++) {
    if ($out[$i] -notmatch '^\|' -or $out[$i] -match '^\|\s*(-+\s*\|\s*)+$') { continue }
    $c = @(Split-IngestRow $out[$i])
    if ($c.Count -eq 6 -and $c[0] -eq $name -and $c[1] -like '(fill in*') {
      $gpus = @($fp.gpus | ForEach-Object { '{0}{1}' -f $_.name, $(if ($_.driver) { " (driver $($_.driver))" } else { '' }) }) -join '; '
      $c[1] = ('{0} {1} ({2})' -f $fp.maker, $fp.model, $Summary.machine)
      $c[2] = ('{0} ({1} cores, {2} threads; AVX2 {3}, AVX-512 {4})' -f $fp.cpu, $fp.physical_cores, $fp.logical_processors, $(if ($fp.avx2) { 'yes' } else { 'no' }), $(if ($fp.avx512f) { 'yes' } else { 'no' }))
      $c[3] = ('{0} GB' -f $fp.ram_gb)
      $c[4] = $gpus
      $c[5] = $Date
      $out[$i] = Join-IngestRow $c
      $changed = $true
    }
  }
  [pscustomobject]@{ Lines = $out; Changed = $changed }
}

# Pure. The reporters' consistency findings (gate id -> problems) lowered onto the gates of a summary:
# PASS becomes FAIL and INFO becomes INVALID, with the first problem added to the headline. Nothing is
# ever raised. Returns the lines to print; the gate objects are changed in place.
function Set-IngestReporterFindings {
  [CmdletBinding()]
  param([Parameter(Mandatory = $true)] $Summary, $Findings)
  $out = New-Object System.Collections.Generic.List[string]
  if ($null -eq $Findings) { return @() }
  foreach ($p in $Findings.PSObject.Properties) {
    $problems = @($p.Value)
    if ($problems.Count -eq 0) { continue }
    foreach ($g in @($Summary.gates | Where-Object { [string]$_.id -eq $p.Name })) {
      $was = [string]$g.status
      $now = $was
      if ($was -eq 'PASS') { $now = 'FAIL' } elseif ($was -eq 'INFO') { $now = 'INVALID' }
      if ($now -ne $was) {
        $g.status = $now
        $g.headline = ('{0}; the ingest reporter found: {1}' -f [string]$g.headline, [string]$problems[0])
        $out.Add(('  reporter: {0} {1} -> {2}: {3}' -f $p.Name, $was, $now, [string]$problems[0]))
      } else {
        $out.Add(('  reporter: {0} stays {1}; findings: {2}' -f $p.Name, $was, ($problems -join ' | ')))
      }
    }
  }
  @($out.ToArray())
}

# Where the evaluation environment's Python is: this checkout, the environment variable WB_EVAL_PYTHON,
# or the main checkout of this repository when this is a worktree. '' when none exists.
function Find-IngestPython {
  [CmdletBinding()]
  param([Parameter(Mandatory = $true)] [string]$Repo)
  $c = New-Object System.Collections.Generic.List[string]
  $c.Add((Join-Path $Repo 'eval\.venv\Scripts\python.exe'))
  if ($env:WB_EVAL_PYTHON) { $c.Add($env:WB_EVAL_PYTHON) }
  try {
    $g = Invoke-PackNative -FilePath 'git' -ArgumentList @('-C', $Repo, 'rev-parse', '--path-format=absolute', '--git-common-dir') -TimeoutSeconds 30
    $common = @($g.Output | Select-Object -First 1)[0]
    if ($common) { $c.Add((Join-Path (Split-Path -Parent ([string]$common)) 'eval\.venv\Scripts\python.exe')) }
  } catch { }
  foreach ($p in $c) { if ($p -and (Test-Path -LiteralPath $p)) { return $p } }
  ''
}

Export-ModuleMember -Function Set-IngestReporterFindings, Find-IngestPython, Get-IngestTableMap, ConvertTo-IngestRedacted, Get-IngestRequiredRoles, Read-IngestAnnotations, Get-IngestStatusCell, Update-IngestGatesLines, Update-IngestRunsBlock, Update-IngestMachines
