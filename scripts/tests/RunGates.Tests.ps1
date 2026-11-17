# Integration test of scripts/gate-pack/run-gates.ps1 with a small fake pack: the runner picks gates
# in order, saves state, writes summary.json and the zip, repeats an INVALID gate, waits for a missing
# interface, skips what is not in the pack, and -Status and -Cleanup work. No audio, no engines.
# Run: powershell -NoProfile -ExecutionPolicy Bypass -File scripts/tests/RunGates.Tests.ps1
# ASCII only; Windows PowerShell 5.1.
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo = (Resolve-Path (Join-Path $here '..\..')).Path
$fails = 0
function Check([string]$Name, [bool]$Ok) {
  if ($Ok) { Write-Host "ok   $Name" } else { Write-Host "FAIL $Name"; $script:fails++ }
}
$ps = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
$tmp = Join-Path ([IO.Path]::GetTempPath()) ('wb-rungates-' + [guid]::NewGuid().ToString('N'))
$pack = Join-Path $tmp 'pack'
New-Item -ItemType Directory -Force -Path (Join-Path $pack 'scripts\gate-pack\gates'), (Join-Path $pack 'scripts\lib') | Out-Null
Copy-Item -Path (Join-Path $repo 'scripts\lib\*') -Destination (Join-Path $pack 'scripts\lib') -Recurse -Force
Copy-Item -LiteralPath (Join-Path $repo 'scripts\gate-pack\run-gates.ps1') -Destination (Join-Path $pack 'run-gates.ps1')

function New-FakeGate([string]$id, [string]$status, [string]$body = '') {
  $s = @(
    'param([string]$CtxFile, [string]$ResultFile)',
    '$lib = Join-Path $PSScriptRoot ''..\..\lib''',
    'Import-Module (Join-Path $lib ''PackCore.psm1'') -Force; Import-Module (Join-Path $lib ''PackGate.psm1'') -Force',
    '$ctx = Read-PackCtx -CtxFile $CtxFile; Initialize-PackGate -Ctx $ctx',
    $body,
    ('$r = New-PackResult -Id "' + $id + '" -Status "' + $status + '" -Headline "fake ' + $id + '" -Rig "fake rig" -Quick ([bool]$ctx.quick)'),
    'Complete-PackGate -Ctx $ctx -Result $r -ResultFile $ResultFile')
  Set-Content -LiteralPath (Join-Path $pack "scripts\gate-pack\gates\$id.ps1") -Value $s -Encoding ASCII
}
function New-Manifest($gates) {
  $m = [ordered]@{ pack_version = 1; gates = @($gates) }
  ($m | ConvertTo-Json -Depth 8) | Set-Content -LiteralPath (Join-Path $pack 'scripts\gate-pack\gates.manifest.json') -Encoding UTF8
}
function G([string]$id, [string]$pri, [int]$order, [string]$group = 'any', $needs = @{}, $deps = @()) {
  [ordered]@{ id = $id; title = "fake $id"; priority = $pri; order = $order; script = "gates/$id.ps1"; group = $group; needs = $needs; interactive = $false; est_minutes = [ordered]@{ low = 1; mid = 1 }; depends_on = $deps; ledger = @() }
}
function Run-Runner([string[]]$ExtraArgs, [string]$Facts = '') {
  $factsFile = Join-Path $tmp 'facts.json'
  if ($Facts) { Set-Content -LiteralPath $factsFile -Value $Facts -Encoding ASCII } else { Set-Content -LiteralPath $factsFile -Value '{"interface":true,"ac":true,"idle":true}' -Encoding ASCII }
  $a = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', (Join-Path $pack 'run-gates.ps1'), '-Role', 'low', '-Unattended', '-UnattendedWaitMinutes', '0', '-NoRestart', '-FactsFile', $factsFile, '-WorkDir', (Join-Path $tmp 'work'), '-BackupRoot', (Join-Path $tmp 'backup')) + $ExtraArgs
  $out = @(& $ps @a 2>&1 | ForEach-Object { "$_" })
  [pscustomobject]@{ Exit = $LASTEXITCODE; Out = $out }
}
function State { Get-Content -LiteralPath (Join-Path $tmp 'work\state.json') -Raw | ConvertFrom-Json }

try {
  # 1. Order, results, summary, zip.
  New-FakeGate 'b-p2' 'PASS'; New-FakeGate 'a-p1' 'PASS'; New-FakeGate 'c-info' 'INFO'; New-FakeGate 'd-fail' 'FAIL'
  New-Manifest @((G 'b-p2' 'P2' 10), (G 'a-p1' 'P1' 20), (G 'c-info' 'P3' 5), (G 'd-fail' 'P2' 30), (G 'e-missing' 'P2' 40))
  $r = Run-Runner @()
  Check 'the runner exits 1 when a gate failed' ($r.Exit -eq 1)
  $st = State
  Check 'P1 ran before P2 and P3' ((@($st.order_used) -join ',') -eq 'a-p1,b-p2,d-fail,c-info')
  Check 'the statuses are saved' ($st.gates.'a-p1'.status -eq 'PASS' -and $st.gates.'d-fail'.status -eq 'FAIL' -and $st.gates.'c-info'.status -eq 'INFO')
  $sum = Get-Content -LiteralPath (Join-Path $tmp 'work\summary.json') -Raw | ConvertFrom-Json
  Check 'summary.json lists the gates and the one not in the pack' (@($sum.gates).Count -eq 4 -and @($sum.not_in_pack).Count -eq 1 -and $sum.not_in_pack[0].id -eq 'e-missing')
  Check 'summary.json says the pack is not emulated and the laptop role' ($null -eq $sum.emulated -and $sum.role -eq 'low')
  $zip = @(Get-ChildItem -LiteralPath $pack -Filter 'gate-results-*.zip')
  Check 'a results zip was written next to the runner' ($zip.Count -eq 1)
  Add-Type -AssemblyName System.IO.Compression.FileSystem
  $z = [System.IO.Compression.ZipFile]::OpenRead($zip[0].FullName)
  $names = @($z.Entries | ForEach-Object { $_.FullName })
  $z.Dispose()
  Check 'the zip holds summary.json, the manifest and the gate results' ($names -contains 'summary.json' -and $names -contains 'ZIP-MANIFEST.json' -and ($names -contains 'work/gates/a-p1/result.json'))
  Check 'the end banner names the zip' (($r.Out -join "`n") -match 'Results zip')
  Check 'the zip name carries the machine, the start of the work folder and nothing else without a label' ($zip[0].Name -match '^gate-results-.+-\d{8}-\d{6}\.zip$')
  # 1b. A second run with another work folder (the mic run has its own) must not replace the first run's zip.
  $firstTime = $zip[0].LastWriteTime; $firstName = $zip[0].Name
  Start-Sleep -Seconds 2
  $facts2 = Join-Path $tmp 'facts.json'
  $out2 = @(& $ps -NoProfile -ExecutionPolicy Bypass -File (Join-Path $pack 'run-gates.ps1') -Role low -Unattended -UnattendedWaitMinutes 0 -NoRestart -FactsFile $facts2 -WorkDir (Join-Path $tmp 'work2') -BackupRoot (Join-Path $tmp 'backup2') -RunLabel 'dryrun-mic' -Only a-p1 2>&1 | ForEach-Object { "$_" })
  $zipsNow = @(Get-ChildItem -LiteralPath $pack -Filter 'gate-results-*.zip')
  Check 'a second run with another work folder writes a second zip' ($zipsNow.Count -eq 2)
  Check 'the first zip is untouched' ((Get-Item -LiteralPath (Join-Path $pack $firstName)).LastWriteTime -eq $firstTime)
  Check 'the second zip carries the run label' (@($zipsNow | Where-Object { $_.Name -like '*-dryrun-mic.zip' }).Count -eq 1)
  foreach ($zz in @($zipsNow | Where-Object { $_.Name -ne $firstName })) { Remove-Item -LiteralPath $zz.FullName -Force }
  Remove-Item -LiteralPath (Join-Path $tmp 'work2') -Recurse -Force -ErrorAction SilentlyContinue

  # 2. A second run does not repeat finished gates; -Retry repeats the FAIL.
  Remove-Item -LiteralPath $zip[0].FullName -Force
  $r2 = Run-Runner @()
  Check 'a second run starts no gate' (-not (($r2.Out -join "`n") -match 'START '))
  $r3 = Run-Runner @('-Retry')
  Check '-Retry runs the failed gate again and only that one' ((($r3.Out | Where-Object { $_ -match 'START ' }) | Measure-Object).Count -eq 1 -and ($r3.Out -join "`n") -match 'START P2 d-fail')

  # 3. INVALID is repeated up to three times.
  Remove-Item -LiteralPath (Join-Path $tmp 'work') -Recurse -Force
  New-FakeGate 'flaky' 'INVALID'
  New-Manifest @((G 'flaky' 'P1' 1))
  $r4 = Run-Runner @()
  $st = State
  Check 'an INVALID gate is tried three times and then left' ($st.gates.flaky.attempts -eq 3 -and $st.gates.flaky.status -eq 'INVALID')

  # 4. A gate that crashes without a result is INVALID with the reason.
  Remove-Item -LiteralPath (Join-Path $tmp 'work') -Recurse -Force
  Set-Content -LiteralPath (Join-Path $pack 'scripts\gate-pack\gates\crash.ps1') -Value @('param([string]$CtxFile, [string]$ResultFile)', 'throw "boom"') -Encoding ASCII
  New-Manifest @((G 'crash' 'P1' 1))
  $r5 = Run-Runner @('-Only', 'crash')
  $st = State
  Check 'a crashed gate is INVALID and the reason says it ended without a result' ($st.gates.crash.status -eq 'INVALID' -and $st.gates.crash.reason -match 'without a result')

  # 4a. An error thrown inside a gate's body is INVALID once and is not repeated in the same run (the next try
  # would throw the same error); a plain INVALID result (section 3) is still tried up to three times.
  Remove-Item -LiteralPath (Join-Path $tmp 'work') -Recurse -Force
  Set-Content -LiteralPath (Join-Path $pack 'scripts\gate-pack\gates\thrower.ps1') -Encoding ASCII -Value @(
    'param([string]$CtxFile, [string]$ResultFile)',
    '$lib = Join-Path $PSScriptRoot ''..\..\lib''',
    'Import-Module (Join-Path $lib ''PackCore.psm1'') -Force; Import-Module (Join-Path $lib ''PackGate.psm1'') -Force',
    '$ctx = Read-PackCtx -CtxFile $CtxFile; Initialize-PackGate -Ctx $ctx',
    'Invoke-PackGateBody -Ctx $ctx -Id ''thrower'' -ResultFile $ResultFile -Body { throw ''boom inside the body'' }')
  New-Manifest @((G 'thrower' 'P1' 1))
  $r5t = Run-Runner @()
  $st = State
  Check 'a gate that throws inside its body is INVALID after one try, with the error as the reason' ($st.gates.thrower.status -eq 'INVALID' -and $st.gates.thrower.attempts -eq 1 -and $st.gates.thrower.reason -match 'boom inside the body')
  $r5u = Run-Runner @()
  Check 'a later run tries it again (up to three tries in all)' ((State).gates.thrower.attempts -eq 2)

  # 4b. A comma list given with -File (-Only a,b / -Skip a,b) arrives as one string and is split.
  Remove-Item -LiteralPath (Join-Path $tmp 'work') -Recurse -Force
  New-FakeGate 'x1' 'PASS'; New-FakeGate 'x2' 'PASS'; New-FakeGate 'x3' 'PASS'
  New-Manifest @((G 'x1' 'P1' 1), (G 'x2' 'P1' 2), (G 'x3' 'P1' 3))
  $r5c = Run-Runner @('-Skip', 'x1,x2')
  $st = State
  Check 'a comma list in -Skip skips every gate in it' ($st.gates.x3.status -eq 'PASS' -and -not $st.gates.PSObject.Properties['x1'] -and -not $st.gates.PSObject.Properties['x2'])

  # 5. A missing interface: the gate waits, the others run, the end says what is waiting.
  Remove-Item -LiteralPath (Join-Path $tmp 'work') -Recurse -Force
  New-FakeGate 'needs-if' 'PASS'; New-FakeGate 'free' 'PASS'
  New-Manifest @((G 'needs-if' 'P1' 1 'interface' @{ interface = $true }), (G 'free' 'P2' 2))
  $r6 = Run-Runner @() '{"interface":false,"ac":true,"idle":true}'
  $st = State
  Check 'the gate that needs the interface did not run and the other did' ($st.gates.free.status -eq 'PASS' -and $st.gates.'needs-if'.status -eq 'PENDING' -and $st.gates.'needs-if'.reason -match 'interface')
  $sum = Get-Content -LiteralPath (Join-Path $tmp 'work\summary.json') -Raw | ConvertFrom-Json
  Check 'the summary lists it as pending' (@($sum.pending | Where-Object { $_.id -eq 'needs-if' }).Count -eq 1)
  Check 'the end banner says the interface is missing' (($r6.Out -join "`n") -match 'interface')

  # 6. With the interface back, the next run finishes it.
  $r7 = Run-Runner @()
  $st = State
  Check 'with the interface back the waiting gate runs' ($st.gates.'needs-if'.status -eq 'PASS')

  # 7. A prompt from a gate is answered through the runner (unattended: such a gate is not started).
  # 8. -Status prints the table and changes nothing.
  $before = (Get-Content -LiteralPath (Join-Path $tmp 'work\state.json') -Raw)
  $s = @(& $ps -NoProfile -ExecutionPolicy Bypass -File (Join-Path $pack 'run-gates.ps1') -Status -WorkDir (Join-Path $tmp 'work') 2>&1 | ForEach-Object { "$_" })
  Check '-Status lists the gates' (($s -join "`n") -match 'needs-if' -and ($s -join "`n") -match 'PASS')
  Check '-Status changes nothing' ($before -eq (Get-Content -LiteralPath (Join-Path $tmp 'work\state.json') -Raw))

  # 9. -Cleanup with nothing changed says so and exits 0.
  $c = @(& $ps -NoProfile -ExecutionPolicy Bypass -File (Join-Path $pack 'run-gates.ps1') -Cleanup -WorkDir (Join-Path $tmp 'work') 2>&1 | ForEach-Object { "$_" })
  Check '-Cleanup with nothing changed says so' (($c -join "`n") -match 'Nothing is left changed' -and $LASTEXITCODE -eq 0)
} finally {
  Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue
}
if ($fails) { Write-Host "$fails failed"; exit 1 }
Write-Host 'RunGates tests passed'
