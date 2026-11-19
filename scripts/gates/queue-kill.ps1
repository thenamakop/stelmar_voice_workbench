<#
.SYNOPSIS
  Phase 4 engine-kill test (plan step 7): two sessions of 30 minutes (English and Hindi in turn; code-switched) recorded
  through the virtual cable on an emulation profile with the hybrid forced (labelled), five CPU (avx2) and five GPU
  (vulkan) batch engines killed from outside at seeded random times, then a copy of each session decoded again by the
  same queue (import) for comparison. PROVISIONAL on an emulation profile; the laptops repeat it.

.DESCRIPTION
  This is day-sim.ps1 -Kill (the one script holds the rig, the gate lock, the quiet-machine rule and the evidence
  writer). The audio comes from: eval\.venv\Scripts\python scripts\gates\make_day_audio_full.py --kill-test
  Checks (scripts\gates\day_report.py --mode kill): no segment lost or repeated, audio order, the job cursor, memory-
  limit deaths reported apart from other failures, the engine peak under 70% of its limit, speech covered in the
  killed run against the uninterrupted copy. WB_GATE_FORCE_HYBRID is set only by this mode and is stated in the evidence.

.EXAMPLE
  powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\gates\queue-kill.ps1 -Profile emu-low
#>
[CmdletBinding()]
param(
  [ValidateSet('emu-low', 'emu-mid')] [string]$Profile = 'emu-low',
  [string]$ProfileData = 'D:\STELMAR-Gate\system-check-19nov\full-emu-low\data',
  [string]$Audio = 'D:\STELMAR-Gate\day-sim\kill-audio'
)
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
& (Join-Path $here 'day-sim.ps1') -Kill -Profile $Profile -ProfileData $ProfileData -Audio $Audio
exit $LASTEXITCODE
