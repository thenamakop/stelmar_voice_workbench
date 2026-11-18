# Tests for scripts/lib/Ingest.psm1 and scripts/ingest-gate-results.ps1: a fixture zip, a failing zip, an
# emulated zip, the VB-CABLE fallback, re-ingest, a damaged zip, and -Preview.
# Run: powershell -NoProfile -ExecutionPolicy Bypass -File scripts/tests/Ingest.Tests.ps1
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

# --- pure parts ---
Check 'a low-baseline-only row needs the low laptop' ((@(Get-IngestRequiredRoles -MachineCell 'low-baseline laptop')) -join ',' -eq 'low')
Check 'a row naming both, or the Legion, needs both' ((@(Get-IngestRequiredRoles -MachineCell 'low-baseline and mid laptops')) -join ',' -eq 'low,mid' -and (@(Get-IngestRequiredRoles -MachineCell 'Legion, `emu-low`')) -join ',' -eq 'low,mid')
Check 'a mid-only row needs the mid laptop' ((@(Get-IngestRequiredRoles -MachineCell 'mid laptop')) -join ',' -eq 'mid')
$r = @(ConvertTo-IngestRedacted -Line @('path C:\Users\Someone\x\y.txt', 'HF_TOKEN=abc123', 'plain'))
Check 'user paths and secret values are removed' ($r[0] -eq 'path C:\Users\<user>\x\y.txt' -and $r[1] -eq 'HF_TOKEN=<redacted>' -and $r[2] -eq 'plain')
$cell = 'PASS (PROVISIONAL) [was: x] ;; gate pack: low/soak PASS (M1, 17 Nov 2026); mid/soak FAIL (M2, 18 Nov 2026)'
$a = Read-IngestAnnotations -Cell $cell
Check 'the annotations of a cell are read back' ($a.Base -eq 'PASS (PROVISIONAL) [was: x]' -and $a.Results['low/soak'].State -eq 'PASS' -and $a.Results['mid/soak'].Machine -eq 'M2')

# --- fixture ---
$tmp = Join-Path ([IO.Path]::GetTempPath()) ('wb-ing-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path $tmp | Out-Null
$fixture = @(
  '# Gate ledger', '', '## Machines', '',
  '| Name | Make and model | CPU | RAM | GPUs and driver versions | First used |', '| --- | --- | --- | --- | --- | --- |',
  '| Legion | Lenovo | i9 | 32 GB | RTX | 8 Nov 2026 |',
  '| Low-baseline laptop | (fill in on arrival) | | 16 GB | | |',
  '| Mid laptop | (fill in on arrival) | | 16 GB | | |', '',
  '## Gates', '',
  '| Phase | Gate item | Machine or profile | Status | Evidence | Date |', '| --- | --- | --- | --- | --- | --- |',
  '| 1 | Soak: 2 hours from VB-CABLE; no gaps | Legion, `emu-low` | PASS (PROVISIONAL) | `docs/evidence/phase-1/soak.txt` | 10 Nov 2026 |',
  '| 3 | Verdicts and tiers on the real laptops | low-baseline and mid laptops | DEFERRED (not before 17 Nov 2026) | gate pack | |',
  '| 2 | Quick-check time on the real low-baseline laptop | low-baseline laptop | DEFERRED | gate pack | |', '',
  '## Open items', '',
  '| Found in | Belongs to | Item | Status | Evidence | Date |', '| --- | --- | --- | --- | --- | --- |',
  '| 1 | gate pack | Re-run Phase 1 on both laptops with a hardware audio interface: every run shows a block | OPEN | `docs/x` | 9 Nov 2026 |',
  '| 2 | gate pack | Measure the zero_gap rates of the recorder on real audio | OPEN | `docs/y` | 12 Nov 2026 |', '')
$gatesMd = Join-Path $tmp 'GATES.md'
[IO.File]::WriteAllText($gatesMd, ($fixture -join "`r`n"), (New-Object Text.UTF8Encoding($false)))
$ledgerSoak = @(
  @{ table = 'gates'; match = 'Soak: 2 hours from VB-CABLE'; kind = 'verdict' },
  @{ table = 'open'; match = 'Re-run Phase 1 on both laptops with a hardware audio interface'; kind = 'verdict'; needs_hardware_interface = $true },
  @{ table = 'open'; match = 'Measure the zero_gap rates of the recorder'; kind = 'info'; needs_hardware_interface = $true })
$ledgerQuick = @(
  @{ table = 'gates'; match = 'Verdicts and tiers on the real laptops'; kind = 'info' },
  @{ table = 'gates'; match = 'Quick-check time on the real low-baseline laptop'; kind = 'verdict' })

function New-RunZip([string]$Name, [string]$Machine, [string]$Role, [string]$Emulated, $Gates, [string]$Created) {
  $work = Join-Path $tmp ("work-" + $Name)
  New-Item -ItemType Directory -Force -Path (Join-Path $work 'gates\soak') | Out-Null
  Set-Content -LiteralPath (Join-Path $work 'gates\soak\gate.log') -Value @('log of C:\Users\Real Person\pack', 'TOKEN=zzz') -Encoding UTF8
  New-Item -ItemType Directory -Force -Path (Join-Path $work 'gates\rename-engines\scratch\renamed') | Out-Null
  [IO.File]::WriteAllBytes((Join-Path $work 'gates\rename-engines\scratch\renamed\wb-engine-avx2.exe.renamed'), [byte[]](77, 90, 0, 0))
  [IO.File]::WriteAllBytes((Join-Path $work 'gates\rename-engines\scratch\x.dmp'), [byte[]](1, 2, 3))
  $summary = [ordered]@{
    pack_version = 1; source_commit = 'abc1234'; machine = $Machine; role = $Role; emulated = $(if ($Emulated) { $Emulated } else { $null })
    quick = [bool]$Emulated; created = $Created; gates = @($Gates); pending = @(); not_in_pack = @(); still_changed = @()
    fingerprint = [ordered]@{ maker = 'Acme'; model = 'Book 14'; cpu = 'Test CPU'; physical_cores = 4; logical_processors = 8; avx2 = $true; avx512f = $false; ram_gb = 15.8; gpus = @([ordered]@{ name = 'Intel UHD'; driver = '31.0.101.1'; integrated = $true }) }
  }
  $zip = Join-Path $tmp ("$Name.zip")
  New-PackZip -WorkDir $work -ZipPath $zip -Summary $summary -CommitText 'abc1234' | Out-Null
  $zip
}
function Gate([string]$Id, [string]$Status, $Ledger, [string]$Rig = 'hardware interface loopback', [string]$Head = 'ok') {
  [ordered]@{ id = $Id; priority = 'P1'; status = $Status; headline = $Head; rig = $Rig; measured = [ordered]@{ rig = $(if ($Rig -match 'VB-CABLE') { 'vb-cable-fallback' } else { 'loopback' }) }; quick = $false; not_at_full_length = $false; ledger = @($Ledger) }
}
$ps = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
function Ingest([string[]]$A) {
  $r = Invoke-PackNative -FilePath $ps -ArgumentList (@('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', (Join-Path $repo 'scripts\ingest-gate-results.ps1'), '-Repo', $tmp, '-GatesMd', $gatesMd, '-EvidenceRoot', (Join-Path $tmp 'evidence')) + $A) -TimeoutSeconds 120
  [pscustomobject]@{ Code = $r.ExitCode; Text = ((@($r.Output) + @($r.Error)) -join "`n") }
}
New-Item -ItemType Directory -Force -Path (Join-Path $tmp 'scripts') | Out-Null
Copy-Item -LiteralPath (Join-Path $repo 'scripts\lib') -Destination (Join-Path $tmp 'scripts\lib') -Recurse
try {
  $orig = Get-Content -LiteralPath $gatesMd -Raw

  # --- 1. a real run on the low laptop: the interface soak passes, the quick check is info ---
  $z1 = New-RunZip 'low1' 'Acme-Book-14-aabbcc' 'low' '' @((Gate 'soak' 'PASS' $ledgerSoak), (Gate 'quick-check' 'INFO' $ledgerQuick 'n/a')) '2026-11-17T10:00:00Z'
  $p = Ingest @('-Zip', $z1, '-Preview')
  Check 'the preview says what it would change' ($p.Code -eq 0 -and $p.Text -match 'preview' -and $p.Text -match 'line\(s\) change')
  Check 'the preview writes nothing' ((Get-Content -LiteralPath $gatesMd -Raw) -eq $orig -and -not (Test-Path (Join-Path $tmp 'docs\evidence')))
  $i1 = Ingest @('-Zip', $z1)
  $t = Get-Content -LiteralPath $gatesMd -Raw
  if ($i1.Code -ne 0) { Write-Host $i1.Text }
  Check 'the ingest ran' ($i1.Code -eq 0)
  Check 'the file keeps its CRLF line endings' ($t.Contains("`r`n") -and -not ($t -match "[^\r]\n"))
  Check 'the soak row names the laptop but stays provisional until the mid laptop has run' ($t -match 'Soak: 2 hours[^\r\n]*PASS \(PROVISIONAL\) ;; gate pack: low/soak PASS \(Acme-Book-14-aabbcc, 17 Nov 2026\); still to run: mid/soak')
  Check 'a DEFERRED info row becomes RESULT IN, owner to read' ($t -match 'Verdicts and tiers on the real laptops[^\r\n]*RESULT IN, owner to read \[was: DEFERRED')
  Check 'an INFO result on a low-only verdict row is not closed as PASS: the owner reads it, the old status is kept' ($t -match 'Quick-check time on the real low-baseline laptop[^\r\n]*RESULT IN, owner to read \[was: DEFERRED' -and $t -notmatch 'Quick-check time on the real low-baseline laptop[^\r\n]*PASS \(gate pack\)')
  Check 'an Open item is annotated, never closed by ingest of one laptop' ($t -match 'Re-run Phase 1 on both laptops[^\r\n]*\| OPEN ;; gate pack: low/soak PASS')
  Check 'the Machines row of the low laptop is filled in' ($t -match 'Low-baseline laptop \| Acme Book 14 \(Acme-Book-14-aabbcc\) \| Test CPU \(4 cores, 8 threads; AVX2 yes, AVX-512 no\) \| 15.8 GB \| Intel UHD \(driver 31.0.101.1\)')
  Check 'the run is listed in the Gate pack runs block' ($t -match '## Gate pack runs' -and $t -match '\| soak \| PASS \| low laptop, Acme-Book-14-aabbcc')
  $ev = Join-Path $tmp 'docs\evidence\gate-pack\Acme-Book-14-aabbcc\gates\soak\gate.log'
  Check 'evidence is copied with user paths and secrets removed' ((Test-Path $ev) -and ((Get-Content -LiteralPath $ev -Raw) -match 'Users\\<user>\\pack' -and (Get-Content -LiteralPath $ev -Raw) -match 'TOKEN=<redacted>'))
  Check 'a renamed engine program or a dump is never copied into the evidence' (@(Get-ChildItem -LiteralPath (Join-Path $tmp 'docs\evidence') -Recurse -File | Where-Object { $_.Extension -in '.renamed', '.dmp', '.exe' }).Count -eq 0)
  Check 'RESULTS.txt is written by the script' (Test-Path (Join-Path $tmp 'docs\evidence\gate-pack\Acme-Book-14-aabbcc\RESULTS.txt'))

  # --- 2. re-ingest of the same zip changes nothing ---
  $before = Get-Content -LiteralPath $gatesMd -Raw
  [void](Ingest @('-Zip', $z1))
  Check 're-ingesting the same zip changes nothing' ((Get-Content -LiteralPath $gatesMd -Raw) -eq $before)

  # --- 3. the mid laptop, a pass: the soak row closes and the open item is still annotated only ---
  $z2 = New-RunZip 'mid1' 'Acme-Book-16-ddeeff' 'mid' '' @((Gate 'soak' 'PASS' $ledgerSoak), (Gate 'quick-check' 'INFO' $ledgerQuick 'n/a')) '2026-11-18T10:00:00Z'
  [void](Ingest @('-Zip', $z2))
  $t = Get-Content -LiteralPath $gatesMd -Raw
  Check 'both laptops passing closes the soak row and keeps the old status' ($t -match 'Soak: 2 hours[^\r\n]*PASS \(gate pack\) \[was: PASS \(PROVISIONAL\)\] ;; gate pack: low/soak PASS[^\r\n]*mid/soak PASS')
  Check 'the open item says the result is in and leaves closing to the owner' ($t -match 'Re-run Phase 1 on both laptops[^\r\n]*RESULT IN: PASS everywhere it is needed, owner to close')

  # --- 4. the VB-CABLE fallback leaves the hardware-interface rows open ---
  $gatesMd2 = Join-Path $tmp 'GATES2.md'
  [IO.File]::WriteAllText($gatesMd2, $orig, (New-Object Text.UTF8Encoding($false)))
  $gatesMd = $gatesMd2
  $z3 = New-RunZip 'fb' 'Acme-Book-14-aabbcc' 'low' '' @((Gate 'soak' 'PASS' $ledgerSoak 'VB-CABLE fallback')) '2026-11-17T12:00:00Z'
  [void](Ingest @('-Zip', $z3))
  $t = Get-Content -LiteralPath $gatesMd2 -Raw
  Check 'a fallback run leaves the Open item exactly OPEN' ($t -match 'Re-run Phase 1 on both laptops[^\r\n]*\| OPEN ;; gate pack: low/soak FALLBACK')
  Check 'a fallback run does not become a PASS of the hardware-interface row' ($t -notmatch 'RESULT IN: PASS everywhere[^\r\n]*Re-run Phase 1' -and $t -notmatch 'Re-run Phase 1[^\r\n]*RESULT IN: PASS')

  # --- 5. a FAIL reopens the phase ---
  [IO.File]::WriteAllText($gatesMd2, $orig, (New-Object Text.UTF8Encoding($false)))
  $z4 = New-RunZip 'fail' 'Acme-Book-14-aabbcc' 'low' '' @((Gate 'soak' 'FAIL' $ledgerSoak 'hardware interface loopback' 'app-only break at 3,171 ms')) '2026-11-17T14:00:00Z'
  $i4 = Ingest @('-Zip', $z4)
  $t = Get-Content -LiteralPath $gatesMd2 -Raw
  Check 'a failing gate marks its row FAIL with the old status kept' ($t -match 'Soak: 2 hours[^\r\n]*FAIL \(gate pack\) \[was: PASS \(PROVISIONAL\)\]')
  Check 'a failing gate adds a REOPENED row to the Open items' ($t -match '\| 1 \| gate pack \| Phase 1 REOPENED: gate soak FAILED on Acme-Book-14-aabbcc[^\r\n]*app-only break')
  Check 'the ingest says plainly that a gate failed' ($i4.Text -match 'GATE FAILED: soak')
  $rows = @(Get-PackLedgerRows -GatesMdPath $gatesMd2)
  Check 'the file still reads as the same two tables (one row more in the Open items)' (@($rows | Where-Object { $_.Table -eq 'open' }).Count -eq 3 -and @($rows | Where-Object { $_.Table -eq 'gates' }).Count -eq 3)

  # --- 6. an emulated run changes no real row ---
  [IO.File]::WriteAllText($gatesMd2, $orig, (New-Object Text.UTF8Encoding($false)))
  $z5 = New-RunZip 'emu' 'Legion' 'low' 'emu-low' @((Gate 'soak' 'PASS' $ledgerSoak 'cable'), (Gate 'quick-check' 'INFO' $ledgerQuick 'n/a')) '2026-11-15T09:00:00Z'
  [void](Ingest @('-Zip', $z5))
  $t = Get-Content -LiteralPath $gatesMd2 -Raw
  $stripped = ($t -split "## Gate pack runs")[0].TrimEnd()
  Check 'an emulated run leaves every row of the two tables as it was' ($stripped -eq $orig.TrimEnd())
  Check 'an emulated run is listed as emulated and not at full length' ($t -match 'emulated: emu-low on the Legion \(not a laptop\)' -and $t -match '-Quick: not at full length')
  Check 'an emulated run goes to its own evidence folder' (Test-Path (Join-Path $tmp 'docs\evidence\gate-pack-dryrun\emu-low\RESULTS.txt'))

  # --- 6b. a native run on the Legion (the microphone test) filed with -AsDryRun is a dry run, never a laptop ---
  [IO.File]::WriteAllText($gatesMd2, $orig, (New-Object Text.UTF8Encoding($false)))
  $z5b = New-RunZip 'mic' 'Legion' 'low' '' @((Gate 'soak' 'PASS' $ledgerSoak 'cable'), (Gate 'quick-check' 'INFO' $ledgerQuick 'n/a')) '2026-11-17T04:00:00Z'
  $i5b = Ingest @('-Zip', $z5b, '-AsDryRun', 'legion-native-mic')
  $t = Get-Content -LiteralPath $gatesMd2 -Raw
  $stripped = ($t -split "## Gate pack runs")[0].TrimEnd()
  Check '-AsDryRun: the run is ingested (exit 0)' ($i5b.Code -eq 0)
  Check '-AsDryRun: every row of the two tables is as it was, and the Machines table is not filled' ($stripped -eq $orig.TrimEnd())
  Check '-AsDryRun: the run is listed as a dry run on the Legion, not a laptop' ($t -match 'dry run: native on the Legion \(not a laptop\), legion-native-mic' -and $t -cnotmatch 'emulated: legion-native-mic' -and $t -notmatch 'low laptop, Legion')
  Check '-AsDryRun: the evidence goes to the dry-run folder under the label' ((Test-Path (Join-Path $tmp 'docs\evidence\gate-pack-dryrun\legion-native-mic\RESULTS.txt')) -and -not (Test-Path (Join-Path $tmp 'docs\evidence\gate-pack\Legion')))
  [IO.File]::WriteAllText($gatesMd2, $orig, (New-Object Text.UTF8Encoding($false)))
  $i5c = Ingest @('-Zip', $z5, '-AsDryRun', 'rerun-x')
  $t = Get-Content -LiteralPath $gatesMd2 -Raw
  Check '-AsDryRun on an emulated zip keeps the profile and uses its own evidence folder' ($i5c.Code -eq 0 -and $t -match 'emulated: emu-low on the Legion' -and (Test-Path (Join-Path $tmp 'docs\evidence\gate-pack-dryrun\rerun-x\RESULTS.txt')))
  $i5d = Ingest @('-Zip', $z5b, '-AsDryRun', 'bad label!')
  Check '-AsDryRun takes only a plain label' ($i5d.Code -ne 0 -and $i5d.Text -match 'short label')

  # --- 7. a damaged zip is refused ---
  $bad = Join-Path $tmp 'bad.zip'
  Copy-Item -LiteralPath $z1 -Destination $bad
  Add-Type -AssemblyName System.IO.Compression
  $zf = [System.IO.Compression.ZipFile]::Open($bad, 'Update')
  try { $e = $zf.GetEntry('summary.json'); $s = New-Object IO.StreamReader($e.Open()); $txt = $s.ReadToEnd(); $s.Dispose(); $e.Delete(); $ne = $zf.CreateEntry('summary.json'); $w = New-Object IO.StreamWriter($ne.Open()); $w.Write($txt); $w.Dispose(); $e2 = $zf.GetEntry('work/gates/soak/gate.log'); $e2.Delete(); $n2 = $zf.CreateEntry('work/gates/soak/gate.log'); $w2 = New-Object IO.StreamWriter($n2.Open()); $w2.Write('tampered'); $w2.Dispose() } finally { $zf.Dispose() }
  $ib = Ingest @('-Zip', $bad)
  Check 'a zip whose file no longer matches its manifest is refused' ($ib.Code -ne 0 -and $ib.Text -match 'does not match its ZIP-MANIFEST')

# --- reporters: findings lower a status, never raise one ---
$sum = '{"gates":[{"id":"quick-check","status":"PASS","headline":"ok"},{"id":"speed-table-igpu","status":"INFO","headline":"cells"},{"id":"soak","status":"PASS","headline":"fine"},{"id":"hashing-1","status":"INVALID","headline":"x"}]}' | ConvertFrom-Json
$find = '{"quick-check":["hybrid choice INCONSISTENT"],"speed-table-igpu":["median differs"],"hashing-1":["something"]}' | ConvertFrom-Json
$out = @(Set-IngestReporterFindings -Summary $sum -Findings $find)
Check 'a PASS with a reporter finding becomes FAIL and says why' (@($sum.gates | Where-Object { $_.id -eq 'quick-check' })[0].status -eq 'FAIL' -and @($sum.gates | Where-Object { $_.id -eq 'quick-check' })[0].headline -match 'INCONSISTENT')
Check 'an INFO with a finding becomes INVALID' (@($sum.gates | Where-Object { $_.id -eq 'speed-table-igpu' })[0].status -eq 'INVALID')
Check 'a gate without a finding is untouched, and an INVALID one is never raised' (@($sum.gates | Where-Object { $_.id -eq 'soak' })[0].status -eq 'PASS' -and @($sum.gates | Where-Object { $_.id -eq 'hashing-1' })[0].status -eq 'INVALID')
Check 'no findings change nothing' (@(Set-IngestReporterFindings -Summary $sum -Findings $null).Count -eq 0)

$py = Find-IngestPython -Repo $repo
if (-not $py) { Write-Host 'skip  pack_reports.py (the evaluation Python was not found)' }
else {
  $w = Join-Path $tmp 'rep-work'
  New-Item -ItemType Directory -Force -Path (Join-Path $w 'gates\speed-table-igpu') | Out-Null
  $recs = @(
    '{"status":"ok","model":"small","clip":"bench-en.wav","mode":"rolling","threads":4,"variant":"vulkan","device_name":"Intel UHD","rtf_median":0.5,"rtf_runs":[0.4,0.5,0.6]}',
    '{"status":"ok","model":"medium","clip":"bench-en.wav","mode":"rolling","threads":4,"variant":"vulkan","device_name":"Intel UHD","rtf_median":0.9,"rtf_runs":[1.2,1.3,1.4]}')
  [IO.File]::WriteAllText((Join-Path $w 'gates\speed-table-igpu\results.jsonl'), (($recs -join "`n") + "`n"), (New-Object Text.UTF8Encoding($false)))
  $ro = Join-Path $tmp 'rep-out'
  $r = Invoke-PackNative -FilePath $py -ArgumentList @('-X', 'utf8', (Join-Path $repo 'scripts\gates\pack_reports.py'), $w, $ro) -TimeoutSeconds 120
  $cj = Read-PackJson -Path (Join-Path $ro 'consistency.json')
  Check 'the speed-table reporter runs and writes its files' ($r.ExitCode -eq 0 -and (Test-Path (Join-Path $ro 'speed-table-igpu.txt')) -and (Test-Path (Join-Path $ro 'quick-vs-full.txt')))
  Check 'a recorded median that is not the median of its runs is a finding; a correct one is not' ($cj -and @($cj.'speed-table-igpu').Count -eq 1 -and (@($cj.'speed-table-igpu')[0]) -match 'medium')
  Check 'a gate that did not run is "not run", not a finding' (((Get-Content -LiteralPath (Join-Path $ro 'speed-table-cpu.txt') -Raw) -match 'not run') -and -not $cj.PSObject.Properties['speed-table-cpu'])
}
} finally { Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue }

# A verdict row is closed only by PASS results (an INFO result must never turn into PASS).
$res = @{ low = [pscustomobject]@{ State = 'INFO'; Machine = 'M'; Date = '15 Nov 2026' } }
$cell = Get-IngestStatusCell -Base 'PROVISIONAL' -Results $res -Required @('low') -Kind 'verdict' -Table 'gates'
Check 'an INFO result on a verdict row of the gates table does not become PASS' ($cell -notmatch '^PASS' -and $cell -match 'RESULT IN, owner to read')
$cell = Get-IngestStatusCell -Base 'OPEN' -Results $res -Required @('low') -Kind 'verdict' -Table 'open'
Check 'an INFO result on a verdict row of the open items does not say PASS everywhere' ($cell -notmatch 'PASS everywhere' -and $cell -match 'RESULT IN, owner to read')
$res = @{ low = [pscustomobject]@{ State = 'PASS'; Machine = 'M'; Date = '15 Nov 2026' } }
Check 'a PASS result still closes a verdict row' ((Get-IngestStatusCell -Base 'PROVISIONAL' -Results $res -Required @('low') -Kind 'verdict' -Table 'gates') -match '^PASS \(gate pack\)')
# A gate whose build the CPU refused (AVX-512) is NOT APPLICABLE for that row: never PASS.
$na = @{ low = [pscustomobject]@{ State = 'NOT APPLICABLE'; Machine = 'M'; Date = '15 Nov 2026' }; mid = [pscustomobject]@{ State = 'NOT APPLICABLE'; Machine = 'M2'; Date = '15 Nov 2026' } }
$cell = Get-IngestStatusCell -Base 'DEFERRED' -Results $na -Required @('low', 'mid') -Kind 'verdict' -Table 'gates'
Check 'a row whose gate was not applicable on every laptop keeps its old status (never PASS)' ($cell -match '^DEFERRED' -and $cell -notmatch '^PASS' -and $cell -match 'NOT APPLICABLE')
$na['mid'] = [pscustomobject]@{ State = 'PASS'; Machine = 'M2'; Date = '15 Nov 2026' }
$cell = Get-IngestStatusCell -Base 'DEFERRED' -Results $na -Required @('low', 'mid') -Kind 'verdict' -Table 'gates'
Check 'one laptop that ran the AVX-512 build and passed closes the row; the other says not applicable' ($cell -match '^PASS \(gate pack\)' -and $cell -match 'low NOT APPLICABLE')
$ing = Get-Content -LiteralPath (Join-Path $here '..\lib\Ingest.psm1') -Raw
Check 'a gate that ran with -Quick on a real laptop counts as INFO for the ledger, never as PASS' ($ing -match 'not_at_full_length' -and $ing -match '\$thisState = ''INFO''')
if ($fails) { Write-Host "$fails failed"; exit 1 }
Write-Host 'Ingest tests passed'
