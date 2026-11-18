# The Gate pack runs block of docs/GATES.md after several dry runs of 17 Nov 2026 (three work folders on one machine):
#  1. an older row of a gate that a later run of the same label has a result for says "superseded by run <id> (pack <commit>)",
#     with the reason given with -Because for the rows that did not pass;
#  2. a run made with -Only gets rows only for the gates it ran or waited on (no WAITING "not run" noise, no NOT IN THIS PACK);
#  3. a native run on the Legion, filed with -AsDryRun, says "dry run: native on the Legion", never "emulated".
# Run: powershell -NoProfile -ExecutionPolicy Bypass -File scripts/tests/IngestLedger.Tests.ps1
# ASCII only; Windows PowerShell 5.1.
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo = (Resolve-Path (Join-Path $here '..\..')).Path
Import-Module (Join-Path $here '..\lib\Ingest.psm1') -Force
Import-Module (Join-Path $here '..\lib\PackCore.psm1') -Force
Import-Module (Join-Path $here '..\lib\PackNative.psm1') -Force
$fails = 0
function Check([string]$Name, [bool]$Ok) {
  if ($Ok) { Write-Host "ok   $Name" } else { Write-Host "FAIL $Name"; $script:fails++ }
}

# ---------- pure functions ----------
function Row([string]$Run, [string]$Gate, [string]$Status, [string]$Who, [string]$Notes) {
  Join-IngestRow @("run M $Run [run $Run]", $Gate, $Status, $Who, $Notes, '`ev`', '17 Nov 2026')
}
$emuWho = 'emulated: emu-low on the Legion (not a laptop)'
$lines = @(
  (Row '2026-11-17T08:59:26Z' 'hashing-1' 'FAIL' $emuWho 'pack aaaaaaa; bad'),
  (Row '2026-11-17T08:59:26Z' 'soak' 'PASS' $emuWho 'pack aaaaaaa; ok'),
  (Row '2026-11-17T12:29:45Z' 'hashing-1' 'PASS' $emuWho 'pack bbbbbbb; ok'),
  (Row '2026-11-17T12:29:45Z' 'soak' 'SKIPPED' $emuWho 'pack bbbbbbb; skipped'),
  (Row '2026-11-17T13:00:00Z' 'hashing-1' 'WAITING' $emuWho 'pack ccccccc; waits'),
  (Row '2026-11-17T04:00:00Z' 'hashing-1' 'PASS' 'dry run: native on the Legion (not a laptop), x' 'pack ddddddd; other label')
)
$u = @(Update-IngestSuperseded -Lines $lines -RunId '2026-11-17T12:29:45Z' -Because 'pack bug fixed in aeb32fb')
$c = @(Split-IngestRow $u[0])[4]
Check 'a failed older row is marked, with the later run, its pack and the reason' ($c -eq 'superseded by run 2026-11-17T12:29:45Z (pack bbbbbbb; pack bug fixed in aeb32fb); pack aaaaaaa; bad')
Check 'the reason is written only for rows that did not pass' ((@(Split-IngestRow $u[1])[4]) -eq 'pack aaaaaaa; ok' -or (@(Split-IngestRow $u[1])[4]) -notmatch 'aeb32fb')
Check 'a later SKIPPED row does not supersede a PASS' ((@(Split-IngestRow $u[1])[4]) -eq 'pack aaaaaaa; ok')
Check 'the latest result is not marked; a later WAITING row does not mark it' ((@(Split-IngestRow $u[2])[4]) -eq 'pack bbbbbbb; ok' -and (@(Split-IngestRow $u[4])[4]) -eq 'pack ccccccc; waits')
Check 'a run of another label is not touched' ((@(Split-IngestRow $u[5])[4]) -eq 'pack ddddddd; other label')
$u2 = @(Update-IngestSuperseded -Lines $u -RunId '2026-11-17T08:59:26Z' -Because 'something else')
Check 'repeating the pass changes nothing' (($u2 -join "`n") -eq ($u -join "`n"))
$u3 = @(Update-IngestSuperseded -Lines $lines)
Check 'without -Because only the later run and its pack are named' ((@(Split-IngestRow $u3[0])[4]) -eq 'superseded by run 2026-11-17T12:29:45Z (pack bbbbbbb); pack aaaaaaa; bad')
$more = @($u) + @(Row '2026-11-17T15:00:00Z' 'hashing-1' 'INFO' $emuWho 'pack eeeeeee; newer')
$u4 = @(Update-IngestSuperseded -Lines $more)
Check 'a still later run moves the mark to it' ((@(Split-IngestRow $u4[2])[4]) -like 'superseded by run 2026-11-17T15:00:00Z (pack eeeeeee); pack bbbbbbb; ok')

$sum = [pscustomobject]@{ order_used = @('a', 'b'); pending = @([pscustomobject]@{ id = 'c' }, [pscustomobject]@{ id = 'd' }) }
$stAll = [pscustomobject]@{ gates = [pscustomobject]@{ a = [pscustomobject]@{ status = 'PASS'; started = 't' }; c = [pscustomobject]@{ status = 'PENDING' }; d = [pscustomobject]@{ status = 'PENDING' } } }
Check 'a run that touched every gate is not scoped' ($null -eq (Get-IngestRunScope -Summary $sum -State $stAll))
$stSome = [pscustomobject]@{ gates = [pscustomobject]@{ a = [pscustomobject]@{ status = 'PASS'; started = 't' }; c = [pscustomobject]@{ status = 'PENDING' }; e = [pscustomobject]@{ status = 'SKIPPED' } } }
Check 'a run with gates it never touched is scoped to the gates it ran or waited on' ((@(Get-IngestRunScope -Summary $sum -State $stSome) -join ',') -eq 'a,b,c')
Check '-Scope overrides' ((@(Get-IngestRunScope -Summary $sum -State $stAll -Explicit 'x,y') -join ',') -eq 'x,y')

# A gate that ran while disturbing programs were running says so in its ledger row; a gate without them says nothing about it.
$sumD = [pscustomobject]@{ created = '2026-11-18T00:00:00Z'; emulated = 'emu-low'; machine = 'M'; source_commit = 'abcdef1234567'; quick = $true
  gates = @([pscustomobject]@{ id = 'soak'; status = 'PASS'; headline = 'ok'; rig = ''; quick = $true; disturbing_programs = @('Steam', 'steamwebhelper') }, [pscustomobject]@{ id = 'panic'; status = 'PASS'; headline = 'ok'; rig = ''; quick = $true; disturbing_programs = @() })
  pending = @(); not_in_pack = @() }
$blk = @(Update-IngestRunsBlock -Lines @('## Gate pack runs') -Summary $sumD -Role 'low' -Date '18 Nov 2026' -EvidenceDir 'ev')
$soakRow = @($blk | Where-Object { $_ -match '\| soak \|' })[0]; $panicRow = @($blk | Where-Object { $_ -match '\| panic \|' })[0]
Check 'a row names the disturbing programs that were running during its gate' ($soakRow -match 'disturbing programs were running: Steam, steamwebhelper')
Check 'a row of a gate with none says nothing about it' ($panicRow -notmatch 'disturbing')

# ---------- the script on three zips ----------
$tmp = Join-Path ([IO.Path]::GetTempPath()) ('wb-ingled-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path (Join-Path $tmp 'scripts') | Out-Null
Copy-Item -LiteralPath (Join-Path $repo 'scripts\lib') -Destination (Join-Path $tmp 'scripts\lib') -Recurse
$gatesMd = Join-Path $tmp 'GATES.md'
$oldIntro = 'One row per gate of each run of `gate-pack/run-gates.ps1`, written by `scripts/ingest-gate-results.ps1`. A row marked "emulated" is a dry run on the Legion (`emu-low` or `emu-mid`, `-Quick`): it proves the pack and the ingest, and is never evidence for the laptops. Real runs also update the rows of the two tables above.'
[IO.File]::WriteAllText($gatesMd, (@('# Gate ledger', '', '## Gates', '', '| Phase | Gate item | Machine or profile | Status | Evidence | Date |', '| --- | --- | --- | --- | --- | --- |', '| 1 | Soak: 2 hours | Legion | PASS (PROVISIONAL) | x | 10 Nov 2026 |', '', '## Open items', '', '| Found in | Belongs to | Item | Status | Evidence | Date |', '| --- | --- | --- | --- | --- | --- |', '', '## Gate pack runs', '', $oldIntro, '', '| Run | Gate | Result | Machine or profile | Notes | Evidence | Date |', '| --- | --- | --- | --- | --- | --- | --- |', '') -join "`r`n"), (New-Object Text.UTF8Encoding($false)))

function G([string]$Id, [string]$Status, [string]$Head = 'ok') { [ordered]@{ id = $Id; priority = 'P1'; status = $Status; headline = $Head; rig = ''; measured = [ordered]@{}; quick = $true; not_at_full_length = $true; ledger = @() } }
function P([string]$Id, [string]$Reason) { [ordered]@{ id = $Id; priority = 'P1'; title = $Id; reason = $Reason; ledger = @() } }
function NewZip([string]$Name, [string]$Created, [string]$Commit, [string]$Emulated, $Gates, $Pending, $Used, $StateGates) {
  $work = Join-Path $tmp ('work-' + $Name)
  New-Item -ItemType Directory -Force -Path (Join-Path $work 'gates\x') | Out-Null
  Set-Content -LiteralPath (Join-Path $work 'gates\x\gate.log') -Value @('log') -Encoding UTF8
  [IO.File]::WriteAllText((Join-Path $work 'state.json'), ((@{ pack_version = 1; gates = $StateGates }) | ConvertTo-Json -Depth 6), (New-Object Text.UTF8Encoding($false)))
  $summary = [ordered]@{ pack_version = 1; source_commit = $Commit; machine = 'Acme-Book-14-aabbcc'; role = 'low'; emulated = $Emulated; quick = $true; created = $Created; order_used = @($Used); gates = @($Gates); pending = @($Pending); not_in_pack = @([ordered]@{ id = 'supervisor-kill'; priority = 'P2'; title = 'sk'; reason = 'not in this pack'; ledger = @() }); still_changed = @() }
  $zip = Join-Path $tmp ($Name + '.zip')
  New-PackZip -WorkDir $work -ZipPath $zip -Summary $summary -CommitText $Commit | Out-Null
  $zip
}
$started = @{ status = 'PASS'; started = '2026-11-17T08:00:00' }
# A: a run of all the gates (part B of the dry run): two hashing gates FAIL, mic-run waits for a person
$zA = NewZip 'A' '2026-11-17T08:59:26Z' 'aaaaaaa1111111' 'emu-low' @((G 'engines-info' 'PASS'), (G 'hashing-1' 'FAIL' 'too short'), (G 'hashing-2-10' 'FAIL' 'too short'), (G 'soak' 'PASS')) @((P 'mic-run' 'needs a person at the keyboard')) @('engines-info', 'hashing-1', 'hashing-2-10', 'soak') @{ 'engines-info' = $started; 'hashing-1' = $started; 'hashing-2-10' = $started; soak = $started; 'mic-run' = @{ status = 'PENDING'; reason = 'needs a person at the keyboard' } }
# B: a later run with -Only engines-info,hashing-1,hashing-2-10 (the re-run on the fixed pack): soak and mic-run were not asked for
$zB = NewZip 'B' '2026-11-17T12:29:45Z' 'bbbbbbb2222222' 'emu-low' @((G 'engines-info' 'PASS'), (G 'hashing-1' 'PASS'), (G 'hashing-2-10' 'INFO'), (G 'full-check' 'SKIPPED' 'skipped under -Quick')) @((P 'soak' 'not run'), (P 'mic-run' 'not run')) @('engines-info', 'hashing-1', 'hashing-2-10') @{ 'engines-info' = $started; 'hashing-1' = $started; 'hashing-2-10' = $started; 'full-check' = @{ status = 'SKIPPED'; reason = 'skipped under -Quick' } }
# C: the microphone run on the Legion's own microphone, no emulation, -Only mic-run
$zC = NewZip 'C' '2026-11-17T14:00:00Z' 'ccccccc3333333' '' @((G 'mic-run' 'PASS'), (G 'full-check' 'SKIPPED' 'skipped under -Quick')) @((P 'soak' 'not run'), (P 'hashing-1' 'not run')) @('mic-run') @{ 'mic-run' = $started; 'full-check' = @{ status = 'SKIPPED' } }

$ps = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
function Ingest([string[]]$A) {
  $r = Invoke-PackNative -FilePath $ps -ArgumentList (@('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', (Join-Path $repo 'scripts\ingest-gate-results.ps1'), '-Repo', $tmp, '-GatesMd', $gatesMd, '-Role', 'low') + $A) -TimeoutSeconds 120
  [pscustomobject]@{ Code = $r.ExitCode; Text = ((@($r.Output) + @($r.Error)) -join "`n") }
}
function RunRows([string]$RunId) {
  $rows = New-Object System.Collections.ArrayList
  foreach ($l in ([IO.File]::ReadAllText($gatesMd) -split "`r?`n")) { if ($l -match '^\|' -and $l.Contains("[run $RunId]")) { [void]$rows.Add(@(Split-IngestRow $l)) } }
  , $rows.ToArray()
}
try {
  $a = Ingest @('-Zip', $zA)
  $b = Ingest @('-Zip', $zB, '-Because', 'pack bug fixed in aeb32fb', '-AsDryRun', 'emu-low-rerun')
  $c = Ingest @('-Zip', $zC, '-AsDryRun', 'legion-native-mic')
  if ($env:WB_TEST_VERBOSE) {
    Write-Host '--- B'; Write-Host $b.Text; Write-Host '--- C'; Write-Host $c.Text; Write-Host '--- block rows'
    foreach ($l in ([IO.File]::ReadAllText($gatesMd) -split "`r?`n")) { if ($l -match '\[run ') { Write-Host $l.Substring(0, [math]::Min(230, $l.Length)) } }
  }
  Check 'the three ingests ran' ($a.Code -eq 0 -and $b.Code -eq 0 -and $c.Code -eq 0)
  if ($b.Code -ne 0) { Write-Host $b.Text }
  if ($a.Code -ne 0) { Write-Host $a.Text }
  if ($c.Code -ne 0) { Write-Host $c.Text }
  Check 'the run that was asked for a few gates says so' ($b.Text -match 'scope: this run was asked for engines-info, hashing-1, hashing-2-10; 4 other gate')
  $rA = RunRows '2026-11-17T08:59:26Z'; $rB = RunRows '2026-11-17T12:29:45Z'; $rC = RunRows '2026-11-17T14:00:00Z'
  function Cell($rows, [string]$Gate, [int]$Col) { @($rows | Where-Object { $_[1] -eq $Gate } | ForEach-Object { $_[$Col] })[0] }

  # 1. superseded
  Check 'the older hashing-1 FAIL says it was superseded, by which run, which pack, and why' ((Cell $rA 'hashing-1' 4) -like 'superseded by run 2026-11-17T12:29:45Z (pack bbbbbbb; pack bug fixed in aeb32fb); *')
  Check 'the older hashing-2-10 FAIL says the same' ((Cell $rA 'hashing-2-10' 4) -like 'superseded by run 2026-11-17T12:29:45Z (pack bbbbbbb; pack bug fixed in aeb32fb); *')
  Check 'the older engines-info PASS is superseded too, but without the reason' ((Cell $rA 'engines-info' 4) -like 'superseded by run 2026-11-17T12:29:45Z (pack bbbbbbb); *')
  Check 'the older soak PASS stays (the later run was not asked for soak)' ((Cell $rA 'soak' 4) -notlike 'superseded*')
  Check 'the later run''s rows are not marked' (@($rB | Where-Object { $_[4] -like 'superseded*' }).Count -eq 0)

  # 2. scope
  Check 'the later run has rows for the 3 gates it ran and no others (no WAITING, no SKIPPED, no NOT IN THIS PACK)' (($rB | ForEach-Object { $_[1] }) -join ',' -eq 'engines-info,hashing-1,hashing-2-10')
  Check 'the full run keeps its WAITING row, with its reason, and its NOT IN THIS PACK row' ((Cell $rA 'mic-run' 2) -eq 'WAITING' -and (Cell $rA 'mic-run' 4) -like '*needs a person at the keyboard*' -and (Cell $rA 'supervisor-kill' 2) -eq 'NOT IN THIS PACK')
  Check 'the microphone run has one row, for mic-run' (($rC | ForEach-Object { $_[1] }) -join ',' -eq 'mic-run')

  # 3. label
  Check 'a native run says "dry run: native on the Legion (not a laptop)" and never "emulated"' ((Cell $rC 'mic-run' 3) -eq 'dry run: native on the Legion (not a laptop), legion-native-mic' -and (($rC | ForEach-Object { $_ -join ' ' }) -join ' ') -notmatch 'emulated')
  Check 'an emulated run still says emulated' ((Cell $rA 'soak' 3) -eq 'emulated: emu-low on the Legion (not a laptop)' -and (Cell $rB 'hashing-1' 3) -eq 'emulated: emu-low on the Legion (not a laptop)')
  $res = Get-Content -LiteralPath (Join-Path $tmp 'docs\evidence\gate-pack-dryrun\legion-native-mic\RESULTS.txt') -Raw
  Check 'the evidence summary of the native run says dry run: native, not EMULATED' ($res -match 'dry run: native on the Legion' -and $res -notmatch 'EMULATED')
  Check 'the console says dry run, native' ($c.Text -match 'DRY RUN, native on the Legion' -and $c.Text -cnotmatch 'EMULATED')
  $md = [IO.File]::ReadAllText($gatesMd)
  Check 'the intro of the block is refreshed (it names the native label and the superseded marks)' ($md -match 'dry run: native' -and $md -match 'superseded by run' -and -not $md.Contains($oldIntro))
  Check 'the Gates and Open items tables are untouched' ($md -match '\| 1 \| Soak: 2 hours \| Legion \| PASS \(PROVISIONAL\) \| x \| 10 Nov 2026 \|')

  # a second ingest of the first zip changes nothing
  $before = [IO.File]::ReadAllText($gatesMd)
  $a2 = Ingest @('-Zip', $zA)
  $setBefore = (@($before -split "`r?`n") | Sort-Object) -join "`n"
  $setAfter = (@([IO.File]::ReadAllText($gatesMd) -split "`r?`n") | Sort-Object) -join "`n"
  $rA2 = RunRows '2026-11-17T08:59:26Z'
  Check 'ingesting the first zip again keeps the marks, with the reason, and changes no row (a run''s rows move to the end of the block)' ($a2.Code -eq 0 -and $setBefore -eq $setAfter -and (Cell $rA2 'hashing-1' 4) -like '*pack bug fixed in aeb32fb*')
} finally {
  Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue
}

if ($fails -gt 0) { Write-Host "$fails failed"; exit 1 }
Write-Host 'IngestLedger tests passed'
