# Tests for scripts/lib/RefVerdict.psm1: the two-reference verdict table and the detector counts.
# Run: powershell -NoProfile -ExecutionPolicy Bypass -File scripts/tests/RefVerdict.Tests.ps1
# ASCII only; Windows PowerShell 5.1.
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
Import-Module (Join-Path $here '..\lib\RefVerdict.psm1') -Force
$fails = 0
function Check([string]$Name, [bool]$Ok) {
  if ($Ok) { Write-Host "ok   $Name" } else { Write-Host "FAIL $Name"; $script:fails++ }
}
$eq = [pscustomobject]@{ aligned = $true; equal = $true }
$ne = [pscustomobject]@{ aligned = $true; equal = $false }
$un = [pscustomobject]@{ aligned = $false; equal = $false }

function V($ac, $aw, $cw) { Get-PackRefVerdict -AppCpal $ac -AppWasapi $aw -CpalWasapi $cw }
Check 'both equal is PASS' ((V $eq $eq $eq).Verdict -eq 'PASS')
Check 'app = WASAPI only is INVALID (cpal reference fault)' ((V $ne $eq $ne).Verdict -eq 'INVALID' -and (V $ne $eq $ne).Class -eq 'cpal-reference-fault')
Check 'app = cpal only is a FAIL at the cpal level' ((V $eq $ne $ne).Verdict -eq 'FAIL' -and (V $eq $ne $ne).Class -eq 'shared-cpal-fault')
Check 'the app differing from two agreeing references is a FAIL' ((V $ne $ne $eq).Verdict -eq 'FAIL' -and (V $ne $ne $eq).Class -eq 'app-differs')
Check 'the app differing from two disagreeing references is INVALID' ((V $ne $ne $ne).Verdict -eq 'INVALID' -and (V $ne $ne $ne).Class -eq 'references-disagree')
Check 'a missing WASAPI reference is INVALID, never a PASS' ((V $eq $null $null).Verdict -eq 'INVALID' -and (V $eq $null $null).Class -eq 'wasapi-reference-missing')
Check 'a missing cpal reference is INVALID' ((V $null $eq $null).Verdict -eq 'INVALID')
Check 'an unaligned comparison counts as missing' ((V $un $eq $eq).Verdict -eq 'INVALID')
Check 'the table lines say all three comparisons' (@((V $eq $eq $eq).Lines).Count -eq 3)
Check 'a compare with no equal field is not comparable' ($null -eq (Test-PackCompareEqual ([pscustomobject]@{ aligned = $true })))

# ---- session.log ----
$log = @(
  '2026-11-12T17:02:02.6184599+05:30 audio_started audio_ms=0',
  '2026-11-12T18:10:31.2979251+05:30 zero_gap audio_ms=3171 start_frame=152168 len_frames=365 len_ms=7.6 rate=48000',
  '2026-11-12T18:10:52.2268903+05:30 zero_gap audio_ms=24071 start_frame=1155377 len_frames=468 len_ms=9.8 rate=48000',
  '2026-11-12T18:11:00.0000000+05:30 repeated_block audio_ms=60000 start_frame=2880000 len_frames=480 lag_frames=96 level_dbfs=-30.0 rate=48000',
  '2026-11-12T18:11:01.0000000+05:30 gap something=else')
$f = @(ConvertFrom-PackSessionLog -Lines $log)
Check 'three firings are read' ($f.Count -eq 3)
Check 'the kind, time and length are read' ($f[0].Kind -eq 'zero_gap' -and $f[0].AudioMs -eq 3171 -and [math]::Abs($f[0].LenMs - 7.6) -lt 0.01)
Check 'a repeated block gets its length from frames and rate' ($f[2].Kind -eq 'repeated_block' -and [math]::Abs($f[2].LenMs - 10.0) -lt 0.01)

# ---- matching ----
$zero = @($f | Where-Object { $_.Kind -eq 'zero_gap' })
$cpal = @([pscustomobject]@{ AudioMs = 3000 }, [pscustomobject]@{ AudioMs = 90000 })
$wasapi = @([pscustomobject]@{ AudioMs = 3010 }, [pscustomobject]@{ AudioMs = 90010 })
$r = Get-PackDetectorRates -Firings $zero -CpalFindings $cpal -WasapiFindings $wasapi -OffsetCpalMs 200 -OffsetWasapiMs 190 -AudioMs 7200000
Check 'a firing near both reference findings is matched by both' ($r.Firings -eq 2 -and $r.Matched -eq 1 -and $r.MatchedBoth -eq 1)
Check 'a firing with no reference finding is a false alarm' ($r.FalseAlarms -eq 1)
Check 'a reference dropout in both captures the detector did not fire for is a miss' ($r.RealMissed -eq 1)
Check 'the rate is per hour of audio' ([math]::Abs($r.PerHour - 1.0) -lt 0.001)
$none = Get-PackDetectorRates -Firings @() -CpalFindings @() -WasapiFindings @() -AudioMs 3600000
Check 'no firings and no findings give zeros' ($none.Firings -eq 0 -and $none.FalseAlarms -eq 0 -and $none.RealMissed -eq 0 -and $none.PerHour -eq 0)
Check 'the verdict line names the detector and the counts' ((Get-PackDetectorLine -Kind 'zero_gap' -Rates $r) -match 'zero_gap: fired 2 time\(s\).*false alarms 1')

if ($fails) { Write-Host "$fails failed"; exit 1 }
Write-Host 'RefVerdict tests passed'
