# Tests for scripts/lib/Rig.psm1: where the gates' player and reference capture run.
# Run: powershell -NoProfile -ExecutionPolicy Bypass -File scripts/tests/Rig.Tests.ps1
# ASCII only; Windows PowerShell 5.1.
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
Import-Module (Join-Path $here '..\lib\GateRun.psm1') -Force
Import-Module (Join-Path $here '..\lib\Rig.psm1') -Force
$fails = 0
function Check([string]$Name, [bool]$Ok) {
  if ($Ok) { Write-Host "ok   $Name" } else { Write-Host "FAIL $Name"; $script:fails++ }
}
function Refuses([scriptblock]$Block, [string]$Pattern) {
  try { & $Block; return $false } catch { return ($_.Exception.Message -match $Pattern) }
}

# ---- Masks ---------------------------------------------------------------------------------
Check 'a hex mask parses' ((ConvertFrom-WbMask '0xf0000') -eq [uint64]0xf0000)
Check 'a mask formats back to hex' ((Format-WbMask ([uint64]0xc000)) -eq '0xc000')
Check 'logical processors are listed as ranges' ((Format-WbLogicalList ([uint64]0xf0000)) -eq '16-19')
Check 'logical processors list pairs and singles' ((Format-WbLogicalList ([uint64]0x5)) -eq '0,2')

# ---- The overlap check: overlapping masks are refused ---------------------------------------
$job = [uint64]0xf0000      # 16-19, the emu-low job
$player = [uint64]0xc000    # 14-15
$ref = [uint64]0x3000       # 12-13
$ok = $true
try { Assert-WbRigPlacement -JobMask $job -PlayerMask $player -ReferenceMask $ref } catch { $ok = $false }
Check 'disjoint masks are accepted' $ok
Check 'a player that shares a core with the job is refused' (Refuses { Assert-WbRigPlacement -JobMask $job -PlayerMask ([uint64]0x30000) -ReferenceMask $ref } 'player.*shares logical processors 16-17 with the job')
Check 'a reference capture that shares a core with the job is refused' (Refuses { Assert-WbRigPlacement -JobMask $job -PlayerMask $player -ReferenceMask ([uint64]0x80000) } 'reference capture.*shares logical processors 19 with the job')
Check 'a player and a reference capture on the same core are refused' (Refuses { Assert-WbRigPlacement -JobMask $job -PlayerMask $player -ReferenceMask ([uint64]0xc000) } 'share logical processors 14-15')
Check 'the refusal says the gate does not start' (Refuses { Assert-WbRigPlacement -JobMask $job -PlayerMask $job -ReferenceMask $ref } 'The gate does not start')
Check 'an empty player mask is refused' (Refuses { Assert-WbRigPlacement -JobMask $job -PlayerMask ([uint64]0) -ReferenceMask $ref } 'player mask is empty')
Check 'a player pinned to every processor is refused' (Refuses { Assert-WbRigPlacement -JobMask $job -PlayerMask ([uint64]4294967295) -ReferenceMask $ref } 'shares logical processors')

# ---- The plan comes from wb-emulate (here a stand-in that prints its JSON) ---------------------
$tmp = Join-Path $env:TEMP ('wb-rig-tests-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $tmp | Out-Null
$fake = Join-Path $tmp 'fake-emulate.cmd'
Set-Content -LiteralPath $fake -Encoding ASCII -Value '@echo {"profile":"emu-low","job_mask":"0xf0000","player_mask":"0xc000","player_logical":[14,15],"reference_mask":"0x3000","reference_logical":[12,13],"strict":true,"note":null}'
$plan = Get-WbRigPlan -Emulate $fake -Profile 'emu-low'
Check 'the plan carries the job, player and reference masks' ($plan.JobMask -eq [uint64]0xf0000 -and $plan.PlayerMask -eq [uint64]0xc000 -and $plan.ReferenceMask -eq [uint64]0x3000 -and $plan.Strict)
Check 'the native profile has no plan' ($null -eq (Get-WbRigPlan -Emulate $fake -Profile 'native'))
$bad = Join-Path $tmp 'bad-emulate.cmd'
Set-Content -LiteralPath $bad -Encoding ASCII -Value "@echo no performance cores outside the mask 1>&2`r`n@exit /b 1"
Check 'a failing wb-emulate stops the gate with its message' (Refuses { Get-WbRigPlan -Emulate $bad -Profile 'emu-low' } 'wb-emulate rig --profile emu-low failed.*no performance cores')

# The real wb-emulate, when it is built: its plan must not overlap any emulation profile.
$realEmu = Join-Path $here '..\..\target\release\wb-emulate.exe'
if (Test-Path -LiteralPath $realEmu) {
  $realEmu = (Resolve-Path -LiteralPath $realEmu).Path
  $okAll = $true
  $union = [uint64]0
  foreach ($p in 'emu-low', 'emu-mid') { $union = $union -bor (Get-WbRigPlan -Emulate $realEmu -Profile $p).JobMask }
  foreach ($p in 'emu-low', 'emu-mid') {
    $rp = Get-WbRigPlan -Emulate $realEmu -Profile $p
    try { Assert-WbRigPlacement -JobMask $rp.JobMask -PlayerMask $rp.PlayerMask -ReferenceMask $rp.ReferenceMask } catch { $okAll = $false }
    if ((($rp.PlayerMask -bor $rp.ReferenceMask) -band $union) -ne 0 -and $rp.Strict) { $okAll = $false }
  }
  Check 'the real wb-emulate places the rig outside both emulation profiles' $okAll
} else {
  Write-Host 'skip real wb-emulate check: target\release\wb-emulate.exe is not built'
}

# ---- Pinning a real process, and reading back what it has ------------------------------------
$self = [System.Diagnostics.Process]::GetCurrentProcess()
$mine = [uint64]$self.ProcessorAffinity.ToInt64()
$lowest = $mine -band (-bnot ($mine - 1))
$child = Start-Process -FilePath (Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe') -ArgumentList @('-NoProfile', '-Command', 'Start-Sleep -Seconds 60') -PassThru -WindowStyle Hidden
try {
  $placed = Set-WbRigPlacement -Process $child -Mask $lowest -Role 'player'
  Check 'a process is pinned and the mask is read back' ($placed.Mask -eq $lowest -and $placed.RequestedMask -eq $lowest)
  Check 'a process is set to HIGH priority class, not realtime' ($placed.Priority -eq 'High')
} finally {
  Stop-Process -Id $child.Id -Force -ErrorAction SilentlyContinue
}

# ---- Late callbacks are shown next to a shared break ------------------------------------------
$play = [pscustomobject]@{
  late_callbacks = 2; max_callback_interval_ms = 160.0
  callback_gaps_ge_20ms = 120; callback_gaps_ge_30ms = 9; callback_gaps_ge_50ms = 2
  late_callback_events = @(
    [pscustomobject]@{ source_ms = 52000.0; at_ms = 53000.0; interval_ms = 160.0; buffer_ms = 100.0 },
    [pscustomobject]@{ source_ms = 90000.0; at_ms = 91000.0; interval_ms = 120.0; buffer_ms = 100.0 })
}
$text = (Get-WbLateCallbackLines -Play $play -BreakSourceMs @(51978.7, 120000.0)) -join "`n"
Check 'the late-callback count, longest gap and gap histogram are shown' ($text -match 'player late callbacks: 2' -and $text -match 'longest gap between callbacks 160.0 ms' -and $text -match 'gaps of 20 ms or more: 120, 30 ms or more: 9, 50 ms or more: 2')
Check 'each late callback is listed with its source position' ($text -match 'late callback at source 52.000 s: 160.0 ms after the previous one \(output buffer 100 ms\)')
Check 'a shared break reports the late callbacks within 500 ms of it' ($text -match 'shared break at source 51.979 s: 1 late player callback\(s\) within 500 ms')
$quiet = [pscustomobject]@{ late_callbacks = 0; max_callback_interval_ms = 21.5; callback_gaps_ge_20ms = 120; callback_gaps_ge_30ms = 0; callback_gaps_ge_50ms = 0; late_callback_events = @() }
$qt = (Get-WbLateCallbackLines -Play $quiet -BreakSourceMs @(3877.0)) -join "`n"
Check 'a break with no late callback says the player never ran its buffer dry' ($qt -match 'no late player callback within 500 ms of it' -and $qt -match 'longest callback gap in the whole run was 21.5 ms' -and $qt -match 'never ran its output buffer dry')
$old = [pscustomobject]@{ underrun_callbacks = 0 }
Check 'an older play.json without the counter says so' (((Get-WbLateCallbackLines -Play $old) -join ' ') -match 'not recorded')

# ---- The go file: created after pinning, also without a plan ------------------------------------
$goFile = Join-Path $tmp 'GO-RIG'
$psExe = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
$c1 = Start-Process -FilePath $psExe -ArgumentList @('-NoProfile', '-Command', 'Start-Sleep -Seconds 60') -PassThru -WindowStyle Hidden
$c2 = Start-Process -FilePath $psExe -ArgumentList @('-NoProfile', '-Command', 'Start-Sleep -Seconds 60') -PassThru -WindowStyle Hidden
try {
  $rigNone = Set-WbRigPins -Plan $null -Player $c1 -Reference $c2 -GoFile $goFile
  Check 'with no plan (native) nothing is pinned but the go file is created' ($null -eq $rigNone -and (Test-Path -LiteralPath $goFile))
  Remove-Item -LiteralPath $goFile -Force
  $own = [uint64]$self.ProcessorAffinity.ToInt64()
  $first = $own -band (-bnot ($own - 1))
  $rest = $own -bxor $first
  $second = $rest -band (-bnot ($rest - 1))
  $plan2 = [pscustomobject]@{ JobMask = [uint64]0; PlayerMask = $first; ReferenceMask = $(if ($second -ne 0) { $second } else { $first }); Strict = $true; Note = $null }
  $pinned = Set-WbRigPins -Plan $plan2 -Player $c1 -Reference $c2 -GoFile $goFile
  Check 'with a plan both processes are pinned at High and the go file appears after' ($pinned.Player.Priority -eq 'High' -and $pinned.Reference.Priority -eq 'High' -and (Test-Path -LiteralPath $goFile))
} finally {
  Stop-Process -Id $c1.Id, $c2.Id -Force -ErrorAction SilentlyContinue
}

# ---- The evidence lines show what the processes really had -------------------------------------
$rig = [pscustomobject]@{
  Plan   = [pscustomobject]@{ Note = $null }
  Job    = [pscustomobject]@{ Role = 'job (the app inside the emulation job)'; ProcessId = 10; Mask = [uint64]0xf0000; Priority = $null }
  Player = [pscustomobject]@{ Role = 'player'; ProcessId = 11; Mask = [uint64]0xc000; Priority = 'High' }
  Reference = [pscustomobject]@{ Role = 'reference capture'; ProcessId = 12; Mask = [uint64]0x3000; Priority = 'High' }
}
$rl = (Get-WbRigLines -Rig $rig -Play ([pscustomobject]@{ affinity_mask = '0xc000' }) -Reference ([pscustomobject]@{ affinity_mask = '0x3000' })) -join "`n"
Check 'the evidence lists the job, player and reference masks with logical processors' ($rl -match 'mask 0xf0000 \(logical processors 16-19\)' -and $rl -match 'player: pid 11, mask 0xc000 \(logical processors 14-15\), priority class High' -and $rl -match 'reference capture: pid 12, mask 0x3000')
Check 'the evidence shows what the player and the reference report themselves' ($rl -match 'player itself reports 0xc000' -and $rl -match 'reference capture itself reports 0x3000')

Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue
if ($fails) { Write-Host "$fails failed"; exit 1 }
Write-Host 'Rig tests passed'
