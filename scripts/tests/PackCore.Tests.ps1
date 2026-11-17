# Tests for scripts/lib/PackCore.psm1: the registry, the rule that picks the next gate, the state
# file, results, the zip file list, and the ledger matches of the real registry.
# Run: powershell -NoProfile -ExecutionPolicy Bypass -File scripts/tests/PackCore.Tests.ps1
# ASCII only; Windows PowerShell 5.1.
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$root = (Resolve-Path (Join-Path $here '..\..')).Path
Import-Module (Join-Path $here '..\lib\PackCore.psm1') -Force
$fails = 0
function Check([string]$Name, [bool]$Ok) {
  if ($Ok) { Write-Host "ok   $Name" } else { Write-Host "FAIL $Name"; $script:fails++ }
}
$tmp = Join-Path ([IO.Path]::GetTempPath()) ('wb-packcore-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path $tmp | Out-Null

function G([string]$id, [string]$pri, [int]$order, [string]$group, $needs = @{}, $deps = @(), [bool]$interactive = $false, [bool]$prepare = $false) {
  [pscustomobject]@{
    id = $id; title = "t $id"; priority = $pri; order = $order; script = "gates/$id.ps1"; group = $group
    needs = [pscustomobject]$needs; interactive = $interactive; prepare = $prepare
    est_minutes = [pscustomobject]@{ low = 10; mid = 10 }; depends_on = $deps; ledger = @()
  }
}
function M($gates) { [pscustomobject]@{ pack_version = 1; gates = @($gates) } }
function St($map) {
  $s = New-PackState -Role low
  foreach ($k in $map.Keys) { Set-PackGateStatus -State $s -Id $k -Fields $map[$k] }
  $s
}
$allFacts = @{ interface = $true; ac = $true; idle = $true }

try {
  # ---- the real registry ----
  $real = Read-PackManifest -Path (Join-Path $root 'scripts\gate-pack\gates.manifest.json')
  Check 'the real registry is valid' (@(Get-PackManifestProblems -Manifest $real).Count -eq 0)
  $hits = @(Test-PackLedgerMatches -Manifest $real -GatesMdPath (Join-Path $root 'docs\GATES.md'))
  $badHits = @($hits | Where-Object { $_.Count -ne 1 })
  Check ('every ledger match finds exactly one row of docs/GATES.md' + $(if ($badHits.Count) { ': ' + (($badHits | ForEach-Object { $_.Gate + ' ' + $_.Count }) -join ', ') } else { '' })) ($badHits.Count -eq 0)
  $realIds = @($real.gates | ForEach-Object { $_.id })
  Check 'the registry has the P1 gates in the owner order' ((@($real.gates | Where-Object { $_.priority -eq 'P1' -and -not $_.prepare } | Sort-Object { [int]$_.order } | ForEach-Object { $_.id }) -join ',') -eq 'quick-check,speed-table-igpu,soak,load-cold,load-settled,panic,webview,kill,hashing-1,network-off')
  Check 'supervisor-kill is P3' (($real.gates | Where-Object { $_.id -eq 'supervisor-kill' }).priority -eq 'P3')
  Check 'every registry id is unique' (@($realIds | Select-Object -Unique).Count -eq $realIds.Count)

  # ---- manifest problems ----
  $bad = M @((G 'a' 'P9' 1 'any'), (G 'a' 'P1' 1 'weird'), (G 'c' 'P1' 1 'cold'), (G 'd' 'P1' 1 'settled'), (G 'e' 'P1' 1 'any' @{} @('nope')))
  $probs = @(Get-PackManifestProblems -Manifest $bad)
  Check 'a bad priority is reported' (@($probs | Where-Object { $_ -match "priority 'P9'" }).Count -eq 1)
  Check 'a duplicate id is reported' (@($probs | Where-Object { $_ -match 'duplicate gate id' }).Count -eq 1)
  Check 'an unknown group is reported' (@($probs | Where-Object { $_ -match "group 'weird'" }).Count -eq 1)
  Check 'a cold gate without a window is reported' (@($probs | Where-Object { $_ -match 'boot_within_min' }).Count -eq 1)
  Check 'a settled gate without an uptime is reported' (@($probs | Where-Object { $_ -match 'uptime_min' }).Count -eq 1)
  Check 'a missing dependency is reported' (@($probs | Where-Object { $_ -match "depends_on 'nope'" }).Count -eq 1)

  # ---- picking the next gate ----
  $m = M @(
    (G 'p2-a' 'P2' 10 'interface' @{ interface = $true })
    (G 'p1-b' 'P1' 20 'any')
    (G 'p1-a' 'P1' 10 'any')
    (G 'p3-a' 'P3' 5 'any')
    (G 'prep' 'P1' 99 'any' @{} @() $false $true)
  )
  $r = Select-PackNextGate -Manifest $m -State (New-PackState) -Facts $allFacts
  Check 'a prepare gate goes first, whatever its priority' ($r.Next.id -eq 'prep')
  $s = St @{ prep = @{ status = 'PASS' } }
  $r = Select-PackNextGate -Manifest $m -State $s -Facts $allFacts
  Check 'then P1 by order' ($r.Next.id -eq 'p1-a')
  $s = St @{ prep = @{ status = 'PASS' }; 'p1-a' = @{ status = 'PASS' }; 'p1-b' = @{ status = 'INFO' } }
  $r = Select-PackNextGate -Manifest $m -State $s -Facts $allFacts
  Check 'then P2, then P3' ($r.Next.id -eq 'p2-a')
  $s = St @{ prep = @{ status = 'PASS' }; 'p1-a' = @{ status = 'PASS' }; 'p1-b' = @{ status = 'PASS' }; 'p2-a' = @{ status = 'FAIL' } }
  $r = Select-PackNextGate -Manifest $m -State $s -Facts $allFacts
  Check 'a FAIL is final (not repeated) without -Retry' ($r.Next.id -eq 'p3-a')
  $r = Select-PackNextGate -Manifest $m -State $s -Facts $allFacts -Retry
  Check '-Retry repeats a FAIL' ($r.Next.id -eq 'p2-a')

  # interface missing
  $r = Select-PackNextGate -Manifest (M @((G 'i' 'P1' 1 'interface' @{ interface = $true }), (G 'x' 'P2' 1 'any'))) -State (New-PackState) -Facts @{ interface = $false; ac = $true }
  Check 'a gate needing the interface waits and the next one runs' ($r.Next.id -eq 'x' -and @($r.Waiting).Count -eq 1 -and $r.Waiting[0].Reasons[0] -match 'interface')
  $r = Select-PackNextGate -Manifest (M @((G 'i' 'P1' 1 'interface' @{ interface = $true }))) -State (New-PackState) -Facts @{}
  Check 'an unknown interface counts as missing' ($null -eq $r.Next)
  $r = Select-PackNextGate -Manifest (M @((G 'i' 'P1' 1 'any' @{ ac = $true }))) -State (New-PackState) -Facts @{ ac = $false }
  Check 'a gate needing AC waits on battery' ($null -eq $r.Next -and $r.Waiting[0].Reasons[0] -match 'AC')
  $r = Select-PackNextGate -Manifest (M @((G 'i' 'P1' 1 'any' @{ ac = $true }))) -State (New-PackState) -Facts @{}
  Check 'an unknown AC state is not a blocker (a desktop has no battery)' ($r.Next.id -eq 'i')

  # cold window and restart
  $cold = M @(
    (G 'load' 'P1' 50 'cold' @{ interface = $true; boot_within_min = 60 })
    (G 'hash' 'P1' 100 'cold' @{ interface = $true; boot_within_min = 30 })
    (G 'soak' 'P1' 40 'interface' @{ interface = $true })
  )
  $r = Select-PackNextGate -Manifest $cold -State (New-PackState) -UptimeMin 5 -Facts $allFacts
  Check 'right after a restart the gate with the shortest window goes first' ($r.Next.id -eq 'hash')
  $s = St @{ hash = @{ status = 'PASS' } }
  $r = Select-PackNextGate -Manifest $cold -State $s -UptimeMin 35 -Facts $allFacts
  Check 'after 35 min the 60-minute gate still goes before the soak' ($r.Next.id -eq 'load')
  $r = Select-PackNextGate -Manifest $cold -State (New-PackState) -UptimeMin 120 -Facts $allFacts
  Check 'a long uptime moves cold gates to waiting and asks for a restart' ($r.Next.id -eq 'soak' -and $r.NeedsRestart -and (@($r.RestartFor) -join ',') -eq 'load,hash')
  $r = Select-PackNextGate -Manifest $cold -State (New-PackState) -UptimeMin 120 -Facts $allFacts -RestartAllowed
  Check 'with a restart allowed the restart goes before a long gate' ($null -eq $r.Next -and $r.NeedsRestart)
  $r = Select-PackNextGate -Manifest $cold -State (New-PackState) -UptimeMin 120 -Facts @{ interface = $false; ac = $true } -RestartAllowed
  Check 'no restart is asked for while the interface is missing' (-not $r.NeedsRestart)
  $prep = M @((G 'prep' 'P1' 1 'any' @{} @() $false $true), (G 'hash' 'P1' 100 'cold' @{ boot_within_min = 30 }))
  $r = Select-PackNextGate -Manifest $prep -State (New-PackState) -UptimeMin 120 -Facts $allFacts -RestartAllowed
  Check 'a prepare gate still runs before the restart' ($r.Next.id -eq 'prep' -and $r.NeedsRestart)

  # settled
  $set = M @((G 'settled' 'P1' 60 'settled' @{ uptime_min = 180; idle = $true }), (G 'x' 'P2' 1 'any'))
  $r = Select-PackNextGate -Manifest $set -State (New-PackState) -UptimeMin 100 -Facts $allFacts
  Check 'a settled gate waits for uptime and says how long' ($r.Next.id -eq 'x' -and $r.WaitUptime.Count -eq 1 -and [math]::Round($r.WaitUptime[0].MinutesLeft) -eq 80)
  $r = Select-PackNextGate -Manifest $set -State (New-PackState) -UptimeMin 200 -Facts $allFacts
  Check 'a settled gate runs after 3 hours' ($r.Next.id -eq 'settled')
  $r = Select-PackNextGate -Manifest $set -State (New-PackState) -UptimeMin 200 -Facts @{ idle = $false }
  Check 'a settled gate waits while the laptop is busy' ($r.Next.id -eq 'x')

  # dependencies, INVALID, only, skip, unattended, not in pack
  $dep = M @((G 'a' 'P1' 1 'any'), (G 'b' 'P1' 2 'any' @{} @('a')))
  $r = Select-PackNextGate -Manifest $dep -State (New-PackState) -Facts $allFacts
  Check 'a gate with an open dependency waits' ($r.Next.id -eq 'a' -and $r.Waiting[0].Reasons[0] -match "waits for gate 'a'")
  $r = Select-PackNextGate -Manifest $dep -State (St @{ a = @{ status = 'FAIL' } }) -Facts $allFacts
  Check 'a failed dependency does not block (the failure is reported on its own)' ($r.Next.id -eq 'b')
  $one = M @((G 'a' 'P1' 1 'any'))
  $r = Select-PackNextGate -Manifest $one -State (St @{ a = @{ status = 'INVALID'; attempts = 1 } }) -Facts $allFacts
  Check 'INVALID is repeated' ($r.Next.id -eq 'a')
  $r = Select-PackNextGate -Manifest $one -State (St @{ a = @{ status = 'INVALID'; attempts = 3 } }) -Facts $allFacts
  Check 'INVALID stops after three attempts' ($null -eq $r.Next)
  $r = Select-PackNextGate -Manifest $one -State (St @{ a = @{ status = 'interrupted'; attempts = 1 } }) -Facts $allFacts
  Check 'an interrupted gate (the laptop restarted under it) is repeated' ($r.Next.id -eq 'a')
  $r = Select-PackNextGate -Manifest $one -State (St @{ a = @{ status = 'interrupted'; attempts = 3 } }) -Facts $allFacts
  Check 'an interrupted gate that used its attempts is left alone in a plain run' ($null -eq $r.Next)
  $r = Select-PackNextGate -Manifest $one -State (St @{ a = @{ status = 'interrupted'; attempts = 3 } }) -Facts $allFacts -Only @('a')
  Check 'an interrupted gate named with -Only is run again, whatever its attempts' ($r.Next.id -eq 'a')
  $r = Select-PackNextGate -Manifest $one -State (St @{ a = @{ status = 'INVALID'; attempts = 3 } }) -Facts $allFacts -Only @('a')
  Check 'an INVALID gate that used its attempts is not looped by -Only' ($null -eq $r.Next)
  $two = M @((G 'a' 'P1' 1 'any'), (G 'b' 'P1' 2 'any'))
  $r = Select-PackNextGate -Manifest $two -State (New-PackState) -Facts $allFacts -Only @('b')
  Check '-Only picks the named gates' ($r.Next.id -eq 'b')
  $r = Select-PackNextGate -Manifest $two -State (New-PackState) -Facts $allFacts -Skip @('a')
  Check '-Skip leaves a gate out' ($r.Next.id -eq 'b')
  $r = Select-PackNextGate -Manifest (M @((G 'n' 'P1' 1 'network' @{} @() $true))) -State (New-PackState) -Facts $allFacts -Unattended
  Check 'an interactive gate is not started unattended' ($null -eq $r.Next -and $r.Waiting[0].Reasons[0] -match 'person')
  $r = Select-PackNextGate -Manifest $two -State (New-PackState) -Facts $allFacts -InPack @('a')
  Check 'a gate whose script is not in the pack is reported, not run' ($r.Next.id -eq 'a' -and (@($r.NotInPack) -join ',') -eq 'b')

  # ---- state, results, JSON ----
  $s = New-PackState -Role mid -Quick $true
  Set-PackGateStatus -State $s -Id 'a' -Fields @{ status = 'PASS'; attempts = 1 }
  Save-PackState -WorkDir $tmp -State $s
  $back = Read-PackState -WorkDir $tmp
  Check 'the state round-trips' ($back['role'] -eq 'mid' -and $back['quick'] -eq $true -and $back['gates']['a']['status'] -eq 'PASS')
  Set-PackGateStatus -State $back -Id 'b' -Fields @{ status = 'FAIL' }
  Check 'a state read back can be changed' ($back['gates'].Contains('b'))
  Check 'no .tmp file is left after a save' (-not (Test-Path -LiteralPath ((Get-PackStatePath $tmp) + '.tmp')))
  Check 'a missing state file reads as null' ($null -eq (Read-PackState -WorkDir (Join-Path $tmp 'nope')))
  $res = New-PackResult -Id 'soak' -Status 'PASS' -Headline 'ok' -Rig 'loopback' -Quick $true -SharedBreaks 2
  $rp = Join-Path $tmp 'res\soak.json'
  Write-PackResult -Path $rp -Result $res
  $rr = Read-PackJson -Path $rp
  Check 'a result round-trips' ($rr.id -eq 'soak' -and $rr.status -eq 'PASS' -and $rr.quick -eq $true -and $rr.shared_breaks -eq 2)
  $threw = $false; try { New-PackResult -Id 'x' -Status 'MAYBE' | Out-Null } catch { $threw = $true }
  Check 'an unknown status is refused' $threw
  Check 'exit codes map to statuses' ((ConvertTo-PackStatus 0) -eq 'PASS' -and (ConvertTo-PackStatus 1) -eq 'FAIL' -and (ConvertTo-PackStatus 3) -eq 'INVALID' -and (ConvertTo-PackStatus 9) -eq 'INVALID')
  $ev = Join-Path $tmp 'ev.txt'
  Set-Content -LiteralPath $ev -Value @('# header', 'verdict: FAIL because x') -Encoding UTF8
  Check 'the verdict line of an evidence file is read' ((Get-PackEvidenceVerdict -Path $ev) -eq 'FAIL' -and $null -eq (Get-PackEvidenceVerdict -Path (Join-Path $tmp 'none.txt')))

  # ---- the zip file list ----
  $w = Join-Path $tmp 'work'
  foreach ($d in 'logs', 'mirror', 'recordings', 'reference', 'reference-wasapi', 'mic-keep', 'backup') { New-Item -ItemType Directory -Force -Path (Join-Path $w $d) | Out-Null }
  Set-Content -LiteralPath (Join-Path $w 'logs\a.log') -Value 'x'
  Set-Content -LiteralPath (Join-Path $w 'state.json') -Value '{}'
  Set-Content -LiteralPath (Join-Path $w 'mirror\part-0001.wav') -Value 'x'
  Set-Content -LiteralPath (Join-Path $w 'reference\part-0001.wav') -Value 'x'
  Set-Content -LiteralPath (Join-Path $w 'mic-keep\speech.wav') -Value 'x'
  Set-Content -LiteralPath (Join-Path $w 'logs\other.wav') -Value 'x'
  $files = @(Get-PackCollectFiles -WorkDir $w | ForEach-Object { $_.FullName.Substring($w.Length).TrimStart('\') })
  Check 'the zip takes logs and state' ($files -contains 'logs\a.log' -and $files -contains 'state.json')
  Check 'the zip never takes the mirror, the references or the kept speech' (@($files | Where-Object { $_ -match '^(mirror|reference|mic-keep|backup|recordings)' }).Count -eq 0)
  Check 'the zip never takes a wav file wherever it is' (@($files | Where-Object { $_ -match '\.wav$' }).Count -eq 0)

  # ---- formatting ----
  Check 'minutes are written as hours and minutes' ((Format-PackMinutes 95) -match '1 ?h' -and (Format-PackMinutes 20) -match '20')
} finally {
    # A line read with Get-Content carries PSPath, PSDrive ... : Windows PowerShell 5.1 ConvertTo-Json would walk
  # them (minutes, then an error). Write-PackJson must write plain values and be quick.
  $lineFile = Join-Path $tmp 'line.txt'
  Set-Content -LiteralPath $lineFile -Value @('NETWORK-OFF RESULT: FAIL', 'other') -Encoding UTF8
  $verdict = (@(Get-Content -LiteralPath $lineFile -Encoding UTF8 | Where-Object { $_ -match '^NETWORK-OFF RESULT' }) | Select-Object -Last 1)
  $jf = Join-Path $tmp 'plain.json'
  $t0 = Get-Date
  Write-PackJson -Path $jf -Value ([ordered]@{ verdict = $verdict; list = @(); one = @('x'); none = $null; nested = [ordered]@{ lines = @($verdict, 'b') }; obj = [pscustomobject]@{ a = 1; b = $verdict }; n = 3; ok = $true })
  $sec = ((Get-Date) - $t0).TotalSeconds
  $back = Get-Content -LiteralPath $jf -Raw -Encoding UTF8 | ConvertFrom-Json
  Check 'Write-PackJson writes a line read with Get-Content as a plain string, and quickly' ($back.verdict -eq 'NETWORK-OFF RESULT: FAIL' -and $sec -lt 5 -and (Get-Content -LiteralPath $jf -Raw) -notmatch 'PSPath|PSDrive|PSProvider')
  Check 'Write-PackJson keeps empty and one-item arrays, nulls, nested objects, numbers and booleans' (@($back.list).Count -eq 0 -and $back.list -is [array] -and @($back.one).Count -eq 1 -and $back.one -is [array] -and $null -eq $back.none -and @($back.nested.lines).Count -eq 2 -and $back.obj.a -eq 1 -and $back.obj.b -eq 'NETWORK-OFF RESULT: FAIL' -and $back.n -eq 3 -and $back.ok -eq $true)
Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue
}

# ---- the results zip name (17 Nov 2026: one name per machine let a second work folder replace the first zip) ----
$zn1 = Get-PackZipName -Machine 'LENOVO-83DF-b8384c' -Created '2026-11-17T17:22:18.1234567+05:30' -RunLabel ''
Check 'the zip name has the machine and the start of the work folder' ($zn1 -eq 'gate-results-LENOVO-83DF-b8384c-20261117-172218.zip')
Check 'the same work folder start and label always give the same name (a resume rewrites its own zip)' ($zn1 -eq (Get-PackZipName -Machine 'LENOVO-83DF-b8384c' -Created '2026-11-17T17:22:18.1234567+05:30'))
Check 'another work folder start gives another name' ($zn1 -ne (Get-PackZipName -Machine 'LENOVO-83DF-b8384c' -Created '2026-11-17T04:00:00.0000000+05:30'))
Check 'the run label is part of the name' ((Get-PackZipName -Machine 'X' -Created '2026-11-17T04:00:00+05:30' -RunLabel 'dryrun-mic') -eq 'gate-results-X-20261117-040000-dryrun-mic.zip')
Check 'a label with spaces and path characters is made safe' ((Get-PackZipName -Machine 'a b' -Created '2026-11-17T04:00:00+05:30' -RunLabel 'x/y z:') -eq 'gate-results-a-b-20261117-040000-x-y-z.zip')
Check 'a missing start time still gives a valid name' ((Get-PackZipName -Machine 'M' -Created '') -match '^gate-results-M-\d{8}-\d{6}\.zip$')

if ($fails) { Write-Host "$fails failed"; exit 1 }
Write-Host 'PackCore tests passed'
