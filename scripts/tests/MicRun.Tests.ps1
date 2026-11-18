# Tests for scripts/lib/MicRun.psm1: the schedule, the microphone choice and the window treatment.
# Run: powershell -NoProfile -ExecutionPolicy Bypass -File scripts/tests/MicRun.Tests.ps1
# ASCII only; Windows PowerShell 5.1.
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
Import-Module (Join-Path $here '..\lib\MicRun.psm1') -Force
$fails = 0
function Check([string]$Name, [bool]$Ok) {
  if ($Ok) { Write-Host "ok   $Name" } else { Write-Host "FAIL $Name"; $script:fails++ }
}

$full = Get-PackMicPlan
Check 'the full run is 2 hours with a 3-minute mute at 40:00 and a 3-minute silence at 80:00' ($full.Seconds -eq 7200 -and $full.MuteAtS -eq 2400 -and $full.MuteForS -eq 180 -and $full.SilenceAtS -eq 4800 -and $full.SilenceForS -eq 180)
$q = Get-PackMicPlan -Quick $true
Check 'the quick run is 15 minutes with a 1-minute mute and a 1-minute silence' ($q.Seconds -eq 900 -and $q.MuteForS -eq 60 -and $q.SilenceForS -eq 60 -and $q.Quick)
foreach ($p in @($full, $q)) {
  $w = @(Get-PackMicWindows -Plan $p)
  Check ("windows of the {0} plan are inside the run and do not overlap" -f $(if ($p.Quick) { 'quick' } else { 'full' })) ($w.Count -eq 2 -and $w[0].EndMs -le $w[1].StartMs -and $w[1].EndMs -le 1000.0 * $p.Seconds -and $w[0].StartMs -gt 0)
}

function In($name, $def) { [pscustomobject]@{ name = $name; is_default = $def } }
$inputs = @((In 'CABLE Output (VB-Audio Virtual Cable)' $true), (In 'Line (2- Focusrite USB Audio)' $false), (In 'Microphone Array (Realtek(R) Audio)' $false), (In 'Headset Microphone (USB)' $true))
$m = Select-PackMicDevice -Inputs $inputs
Check 'a microphone name wins, and a default one among them is preferred' ($m.name -eq 'Headset Microphone (USB)')
$m = Select-PackMicDevice -Inputs @((In 'CABLE Output (VB-Audio Virtual Cable)' $true), (In 'Microphone Array (Realtek(R) Audio)' $false))
Check 'a virtual cable is never the microphone' ($m.name -eq 'Microphone Array (Realtek(R) Audio)')
$m = Select-PackMicDevice -Inputs @((In 'Capture 1 (Some Device)' $true), (In 'Capture 2 (Other)' $false))
Check 'without a microphone name the default input is used' ($m.name -eq 'Capture 1 (Some Device)')
Check 'only a cable and an interface: no microphone' ($null -eq (Select-PackMicDevice -Inputs @((In 'CABLE Output (VB-Audio Virtual Cable)' $true), (In 'Line (2- Focusrite USB Audio)' $false))))

function Fire($k, $ms, $len) { [pscustomobject]@{ Kind = $k; AudioMs = $ms; LenMs = $len } }
$win = @(Get-PackMicWindows -Plan $q)
# App clock = source clock + 4000 ms.
$f = @((Fire 'zero_gap' (300000 + 4000 + 100) 200), (Fire 'repeated_block' (540000 + 4000 + 20000) 50), (Fire 'zero_gap' 100000 100), (Fire 'zero_gap' 700000 100))
$t = @(Get-PackWindowTreatment -Firings $f -Windows $win -OffsetMs 4000)
Check 'one zero_gap inside the mute window' ($t[0].Window -eq 'digital mute' -and $t[0].zero_gap -eq 1 -and $t[0].repeated_block -eq 0)
Check 'one repeated_block inside the room silence' ($t[1].repeated_block -eq 1 -and $t[1].zero_gap -eq 0)
Check 'the other two firings are outside both windows' ($t[2].Kind -eq 'rest' -and $t[2].zero_gap -eq 2)
$t = @(Get-PackWindowTreatment -Firings @() -Windows $win -OffsetMs 0)
Check 'no firings: all zeros' ($t.Count -eq 3 -and ($t | ForEach-Object { $_.zero_gap + $_.repeated_block } | Measure-Object -Sum).Sum -eq 0)
$edge = @(Get-PackWindowTreatment -Firings @((Fire 'zero_gap' 299000 2500)) -Windows $win -OffsetMs 0)
Check 'a firing that starts before a window and reaches into it counts for the window' ($edge[0].zero_gap -eq 1)
$lines = @(Get-PackWindowLines -Treatment (Get-PackWindowTreatment -Firings $f -Windows $win -OffsetMs 4000))
Check 'the lines name each window and warn about firings in the scripted windows' (($lines -join "`n") -match 'digital mute' -and ($lines -join "`n") -match 'false alarms to read' -and ($lines -join "`n") -match 'outside both windows')

# The part of the gate that builds the evidence file and the result runs only after 15 minutes of recording: it is
# run here with synthetic input (a loop variable once overwrote the list it was adding to, and three real runs were lost).
$repoRoot = (Resolve-Path (Join-Path $here '..\..')).Path
Import-Module (Join-Path $repoRoot 'scripts\lib\PackCore.psm1') -Force
$tmpMic = Join-Path ([IO.Path]::GetTempPath()) ('wb-micres-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path $tmpMic | Out-Null
try {
  $cond = New-Object System.Collections.Generic.List[string]
  $cond.Add('audio software running: none'); $cond.Add('speaker volume set to 50 percent (put back at the end)')
  $wlog = New-Object System.Collections.Generic.List[object]
  $wlog.Add([ordered]@{ event = 'mute on'; ok = $true; elapsed_s = 300.4; at = 'x' }); $wlog.Add([ordered]@{ event = 'mute off'; ok = $false; elapsed_s = 360.2; at = 'x' })
  $rvx = [pscustomobject]@{ Verdict = 'PASS'; Class = 'equal'; Reason = 'the app equals both references'; Lines = @('  app against the cpal reference: equal', '  app against the WASAPI reference: equal') }
  $detx = [pscustomobject]@{ Lines = @('  detector zero_gap: fired 1 time(s)', '  detector repeated_block: fired 0 time(s)'); Rates = [ordered]@{ zero_gap = [ordered]@{ Firings = 1 } } }
  $tr = @(Get-PackWindowTreatment -Firings $f -Windows $win -OffsetMs 4000)
  $ctxx = [pscustomobject]@{ machine = 'TESTBOX' }
  $res = New-PackMicRunResult -Ctx $ctxx -GateDir $tmpMic -Run 'D:\run' -Profile 'laptop' -Minutes 15 -Quick $true -MicName 'Microphone Array (Realtek(R) Audio)' -Speaker 'Speakers (Realtek(R) Audio)' -InputInfo '48000 Hz, 2 channel(s), f32' -Cond $cond.ToArray() -Plan $q -WindowLog $wlog.ToArray() -RefVerdict $rvx -Detectors $detx -Treatment $tr -Overruns 0 -Xruns 0 -Status 'PASS' -Why 'the app equals both references' -Threshold 'x'
  $txt = Get-Content -LiteralPath (Join-Path $tmpMic 'mic-run.txt') -Raw -Encoding UTF8
  Check 'the mic-run result is PASS with the evidence file written' ($res.status -eq 'PASS' -and $res.evidence.Count -eq 2 -and (Test-Path -LiteralPath (Join-Path $tmpMic 'mic-run.txt')))
  Check 'the evidence file holds the conditions, window log, reference lines, detector lines, window lines and the verdict' ($txt -match 'audio software running' -and $txt -match 'window log: mute on at 300.4' -and $txt -match 'FAILED to change the mute' -and $txt -match 'app against the WASAPI reference: equal' -and $txt -match 'detector zero_gap: fired 1' -and $txt -match 'what each detector did in each scripted window' -and $txt -match 'digital mute' -and $txt -match 'verdict: PASS')
  Check 'the result carries the notes, the windows and the detector rates' ($res.notes.Count -eq 2 -and @($res.measured.windows).Count -eq 3 -and $res.measured.detectors.zero_gap.Firings -eq 1 -and $res.measured.overrun_samples -eq 0)
  $jf = Join-Path $tmpMic 'result.json'
  Write-PackResult -Path $jf -Result $res
  Check 'the result can be written as JSON' ((Get-Content -LiteralPath $jf -Raw | ConvertFrom-Json).id -eq 'mic-run')
  $res2 = New-PackMicRunResult -Ctx $ctxx -GateDir $tmpMic -Run 'D:\run' -Profile 'laptop' -Minutes 15 -Quick $true -MicName 'mic' -Plan $q -RefVerdict $rvx -Treatment @() -Status 'FAIL' -Why 'x'
  Check 'it also works with no detectors, no conditions, no window log and no treatment' ($res2.status -eq 'FAIL')
} finally { Remove-Item -LiteralPath $tmpMic -Recurse -Force -ErrorAction SilentlyContinue }

# The gate script: gate contract, never unattended or emulated, puts the mute and the volume back, uses both references.
$g = Get-Content -LiteralPath (Join-Path $here '..\gate-pack\gates\mic-run.ps1') -Raw
Check 'mic-run.ps1 uses the gate contract' ($g -match 'Invoke-PackGateBody -Ctx \$ctx -Id ''mic-run''')
Check 'mic-run.ps1 refuses an emulation profile and an unattended run' ($g -match "-ne 'laptop'" -and $g -match 'unattended')
Check 'mic-run.ps1 records the mute and the volume as changes and puts them back in a finally block' ($g -match "-Kind 'mic-mute'" -and $g -match "-Kind 'render-volume'" -and $g -match 'finally' -and $g -match 'state'', \$muteBefore')
Check 'mic-run.ps1 records with both references and compares all three pairs' ($g -match 'capture-wasapi' -and $g -match 'compare-refs.json' -and $g -match 'Get-PackRefVerdict')
Check 'mic-run.ps1 keeps the speech recording under work\audio, which is never zipped' ($g -match "work_dir\) 'audio'")
Check 'mic-run.ps1 is ASCII only' (-not ($g -cmatch '[^\x00-\x7F]'))
$m = Get-Content -LiteralPath (Join-Path $here '..\gate-pack\gates.manifest.json') -Raw | ConvertFrom-Json
foreach ($id in 'mic-run', 'rename-engines') { Check "$id is marked interactive, so an unattended run leaves it waiting" ([bool](@($m.gates | Where-Object { $_.id -eq $id })[0].interactive)) }
$rg = Get-Content -LiteralPath (Join-Path $here '..\gate-pack\run-gates.ps1') -Raw
Check 'run-gates.ps1 blocks sleep and screen-off with SetThreadExecutionState' ($rg -match 'SetThreadExecutionState' -and $rg -match '2147483651')
$pc = Get-Content -LiteralPath (Join-Path $here '..\lib\PackCore.psm1') -Raw
Check 'the render volume is put back on the playback side of the endpoint' ($pc -match "'--flow', 'render', '--level'")

Check 'the runner imports CableRig itself (the import inside LoopbackRig is private to that module), so the interface fact is not silently false' ($rg -match 'Import-Module \(Join-Path \$libDir ''CableRig.psm1''\)')
Check 'the runner removes a stale gate lock only when its process is dead and the gate wrote it' ($rg -match 'stale lock of gate' -and $rg -match 'Get-Process -Id \(\[int\]\(\(\$lp\[0\]')
$st = Get-Content -LiteralPath (Join-Path $here '..\lib\SpeedTable.psm1') -Raw
Check 'under an emulation profile the speed tables take thread counts from the emulated processors, not the host fingerprint' ($st -match 'logical_processors_allowed' -and $st -match '-PhysicalCores \$physCores -LogicalProcessors \$logProcs')

$rgm = Get-Content -LiteralPath (Join-Path $here '..\lib\RecorderGates.psm1') -Raw
Check 'a refusal in a recorder gate is found case-sensitively, so the evidence line "overlay tools closed: yes" is not taken for a refusal' ($rgm -match '-cmatch ''\^\(RIG REFUSED' -and $rgm -notmatch '-match ''\^\(RIG REFUSED')

Check 'the runner marks the gates the manifest skips under -Quick as SKIPPED, unless they are named with -Only' ($rg -match 'skipped \$gid under -Quick' -and $rg -match 'Only -contains \$gid')

Check 'the results zip and the ingested evidence never carry programs or libraries (engine copies from the work folders)' (($pc -match 'lock\|exe\|dll\|pdb\|sys') -and ((Get-Content -LiteralPath (Join-Path $here '..\ingest-gate-results.ps1') -Raw) -match 'never evidence'))

$sm = Get-Content -LiteralPath (Join-Path $here '..\gate-pack-native-smoke.ps1') -Raw
Check 'the native smoke runs the laptop profile, writes no zip, never calls the ingest, and leaves the audible mic run out by default' ($sm -match "'-Profile', 'laptop'" -and $sm -match "'-NoZip'" -and $sm -notmatch 'ingest-gate-results' -and $sm -match 'if \(-not \$IncludeMic\) \{ \$skip.Add\(''mic-run''\)')
Check 'the native smoke is ASCII only' (-not ($sm -cmatch '[^\x00-\x7F]'))

if ($fails) { Write-Host "$fails failed"; exit 1 }
Write-Host 'MicRun tests passed'
