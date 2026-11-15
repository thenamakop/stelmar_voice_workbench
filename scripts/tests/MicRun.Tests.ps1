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

$sm = Get-Content -LiteralPath (Join-Path $here '..\gate-pack-native-smoke.ps1') -Raw
Check 'the native smoke runs the laptop profile, writes no zip, never calls the ingest, and leaves the audible mic run out by default' ($sm -match "'-Profile', 'laptop'" -and $sm -match "'-NoZip'" -and $sm -notmatch 'ingest-gate-results' -and $sm -match 'if \(-not \$IncludeMic\) \{ \$skip.Add\(''mic-run''\)')
Check 'the native smoke is ASCII only' (-not ($sm -cmatch '[^\x00-\x7F]'))

if ($fails) { Write-Host "$fails failed"; exit 1 }
Write-Host 'MicRun tests passed'
