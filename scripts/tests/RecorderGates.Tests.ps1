# Tests for the pure helpers of scripts/lib/RecorderGates.psm1.
# Run: powershell -NoProfile -ExecutionPolicy Bypass -File scripts/tests/RecorderGates.Tests.ps1
# ASCII only; Windows PowerShell 5.1.
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
Import-Module (Join-Path $here '..\lib\RecorderGates.psm1') -Force
$fails = 0
function Check([string]$Name, [bool]$Ok) {
  if ($Ok) { Write-Host "ok   $Name" } else { Write-Host "FAIL $Name"; $script:fails++ }
}
$o = Get-PackRecorderOutcome -ExitCode 0 -EvidenceLines @('verdict: PASS')
Check 'exit 0 is PASS and carries the verdict line' ($o.Status -eq 'PASS' -and $o.VerdictLine -eq 'verdict: PASS')
Check 'exit 1 is FAIL' ((Get-PackRecorderOutcome -ExitCode 1 -EvidenceLines @()).Status -eq 'FAIL')
Check 'exit 3 is INVALID' ((Get-PackRecorderOutcome -ExitCode 3 -EvidenceLines @()).Status -eq 'INVALID')
$fc = @(Get-PackFailedChecks -EvidenceLines @('checks:', '  [x] zero ring-buffer overruns', '  [ ] model files were hashed while the recording ran: 6 files, 4.76 GB, 4.5 s of hashing inside the recording (at least 10 s)', '  [ ] second failing check', 'verdict: FAIL'))
Check 'the failed checks of an evidence file are listed, the passed ones are not' ($fc.Count -eq 2 -and $fc[0] -match '^model files were hashed' -and $fc[1] -eq 'second failing check')
Check 'an evidence file with no failed check lists none' (@(Get-PackFailedChecks -EvidenceLines @('  [x] a', 'verdict: PASS')).Count -eq 0)
Check 'any other exit is INVALID with the code named' ((Get-PackRecorderOutcome -ExitCode 7 -EvidenceLines @()).VerdictLine -match 'exit code 7')

Check 'the run folder is read from the evidence' ((Get-PackRunDirFromEvidence -EvidenceLines @('x', 'audio and reports: C:\w\soak-laptop-1 (not in the repository)')) -eq 'C:\w\soak-laptop-1')
Check 'no run folder gives an empty string' ((Get-PackRunDirFromEvidence -EvidenceLines @('x')) -eq '')

Check 'a hardware loopback is recognised' ((Get-PackRigKindFromEvidence -EvidenceLines @("  rig: HARDWARE LOOPBACK on the audio interface 'Scarlett'")) -eq 'loopback')
Check 'the fallback banner wins' ((Get-PackRigKindFromEvidence -EvidenceLines @('*** FALLBACK RIG: VB-CABLE, NOT A HARDWARE AUDIO INTERFACE LOOPBACK ***')) -eq 'vb-cable-fallback')
Check 'otherwise it is a plain cable' ((Get-PackRigKindFromEvidence -EvidenceLines @('cable rig')) -eq 'vb-cable')

$cmp = '{"segments":[{"aligned":true,"offset_frames":4800},{"aligned":true,"offset_frames":9600}]}' | ConvertFrom-Json
Check 'the offset is the first aligned segment, in ms' ([math]::Abs((Get-PackCompareOffsetMs -Compare $cmp -Rate 48000) - 100.0) -lt 0.001)
$cmp2 = '{"segments":[{"aligned":false,"offset_frames":null}]}' | ConvertFrom-Json
Check 'no aligned segment gives null' ($null -eq (Get-PackCompareOffsetMs -Compare $cmp2 -Rate 48000))
Check 'no compare gives null' ($null -eq (Get-PackCompareOffsetMs -Compare $null))

$scan = '{"blocks":[{"audio_ms":1000.5}],"zero_gaps":[{"audio_ms":20},{"audio_ms":30}]}' | ConvertFrom-Json
Check 'repeated blocks come from blocks' (@(Get-PackScanFindings -Scan $scan -Kind repeated_block).Count -eq 1)
Check 'zero gaps come from zero_gaps' (@(Get-PackScanFindings -Scan $scan -Kind zero_gap).Count -eq 2)
Check 'no scan gives nothing' (@(Get-PackScanFindings -Scan $null -Kind zero_gap).Count -eq 0)

# A long run has a new offset after every unmatched stretch: each finding takes the offset of the segment that covers it.
$cmp = '{"segments":[{"aligned":true,"offset_frames":48000,"reference_start_frame":0,"reference_end_frame":480000},{"aligned":true,"offset_frames":96000,"reference_start_frame":480000,"reference_end_frame":960000}]}' | ConvertFrom-Json
$mapped = @(Get-PackAppTimeFindings -Findings @([pscustomobject]@{ Kind = 'zero_gap'; AudioMs = 1000 }, [pscustomobject]@{ Kind = 'zero_gap'; AudioMs = 15000 }, [pscustomobject]@{ Kind = 'zero_gap'; AudioMs = 25000 }) -Compare $cmp -Rate 48000)
Check 'findings take the offset of the segment that covers them (1 s after the first, 2 s after the second, the first for one outside both)' ($mapped[0].AudioMs -eq 2000 -and $mapped[1].AudioMs -eq 17000 -and $mapped[2].AudioMs -eq 26000)
Check 'no compare report: findings keep their time' ((@(Get-PackAppTimeFindings -Findings @([pscustomobject]@{ Kind = 'zero_gap'; AudioMs = 500 }) -Compare $null))[0].AudioMs -eq 500)

if ($fails) { Write-Host "$fails failed"; exit 1 }
Write-Host 'RecorderGates tests passed'
