# Tests for scripts/lib/QuickCheck.psm1: the quick-check verdict from a report, capability.json and cells.
# Run: powershell -NoProfile -ExecutionPolicy Bypass -File scripts/tests/QuickCheck.Tests.ps1
# ASCII only; Windows PowerShell 5.1.
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
Import-Module (Join-Path $here '..\lib\QuickCheck.psm1') -Force
$fails = 0
function Check([string]$Name, [bool]$Ok) {
  if ($Ok) { Write-Host "ok   $Name" } else { Write-Host "FAIL $Name"; $script:fails++ }
}

function New-Cap([double]$Duration = 410, [bool]$Passed = $true, [double]$HiRtf = 0.69, [bool]$Partial = $true) {
  $json = @{
    check = @{ kind = 'quick'; partial = $Partial; duration_s = $Duration; bar_s = 480.0 }
    decision = @{
      tier = 'C'
      config = @{ label = 'Intel(R) UHD Graphics (Vulkan)'; config = @{ kind = 'gpu'; index = 0 } }
      languages = @{
        en = @{ rolling = @{ model = 'turbo'; rtf = 0.47; meets_bar = $true; ready_after_stop_min = 4.7 } }
        hi = @{ rolling = @{ model = 'turbo'; rtf = $HiRtf; meets_bar = ($HiRtf -le 0.75); ready_after_stop_min = 6.9 } }
      }
      verdict = @('Record + post-process.')
    }
    recorder_safety = @(@{ config = @{ kind = 'gpu'; index = 0 }; passed = $Passed; duration_s = 208.6; overruns = 0; driver_xruns = 0 })
    measurements = @(@{ lang = 'hi'; config = @{ kind = 'gpu'; index = 0 }; rtf = $HiRtf })
  } | ConvertTo-Json -Depth 8
  $json | ConvertFrom-Json
}
$report = ('{"mode":"quick","error":null,"stopped":null,"wall_seconds":409.8}' | ConvertFrom-Json)
$cells = @(
  [pscustomobject]@{ Command = 'bench'; Started = [datetime]'2026-11-14T10:00:00Z'; Seconds = 40.0; Model = 'turbo'; Lang = 'hi'; Device = 'gpu'; Threads = 2; Mode = 'rolling'; Clip = 'bench-hi.wav' },
  [pscustomobject]@{ Command = 'bench'; Started = [datetime]'2026-11-14T10:01:00Z'; Seconds = 25.5; Model = 'turbo'; Lang = 'en'; Device = 'gpu'; Threads = 2; Mode = 'rolling'; Clip = 'bench-en.wav' },
  [pscustomobject]@{ Command = 'info'; Started = [datetime]'2026-11-14T09:59:00Z'; Seconds = 1.0; Model = ''; Lang = ''; Device = ''; Threads = $null; Mode = ''; Clip = '' })

$v = Get-QuickCheckVerdict -Report $report -Capability (New-Cap) -Cells $cells
Check 'a check within the bar with a passed safety run is PASS' ($v.Status -eq 'PASS')
Check 'a partial check is reported, not failed' ($v.Status -eq 'PASS' -and (($v.Lines -join "`n") -match 'partial: yes'))
Check 'cells are counted and summed (info is not a cell)' ($v.Numbers.cells -eq 2 -and $v.Numbers.cells_total_seconds -eq 65.5 -and $v.Numbers.longest_cell_seconds -eq 40)
Check 'the tier and the per-language numbers are read out' ((($v.Lines -join "`n") -match 'tier: C') -and (($v.Lines -join "`n") -match 'hi: turbo, RTF 0.690'))

$v = Get-QuickCheckVerdict -Report $report -Capability (New-Cap -Duration 520) -Cells $cells
Check 'a check over the 480 s bar is FAIL and says so' ($v.Status -eq 'FAIL' -and $v.Headline -match '520 s' -and $v.Headline -match '480')

$v = Get-QuickCheckVerdict -Report $report -Capability (New-Cap -Passed $false) -Cells $cells
Check 'a failed recorder safety run is FAIL' ($v.Status -eq 'FAIL' -and $v.Headline -match 'recorder safety')

$v = Get-QuickCheckVerdict -Report $report -Capability (New-Cap -HiRtf 2.1) -Cells $cells
Check 'a graphics adapter slower than 1.5x on Hindi is flagged' ((($v.Lines -join "`n") -match 'slower than 1.5x real time: YES') -and $v.Numbers.gpu_hindi_slower_than_1_5 -eq $true)

$v = Get-QuickCheckVerdict -Report $report -Capability (New-Cap -Duration 5692) -Cells $cells -BarSeconds 7200
Check 'a full check of 5692 s passes against the 7200 s bar the caller passed, although capability.json says bar_s 480' ($v.Status -ne 'FAIL' -and (($v.Lines -join "`n") -match 'against the 7200 s bar'))

$v = Get-QuickCheckVerdict -Report $null -Capability $null -Cells @()
Check 'no report and no capability.json is INVALID, not FAIL' ($v.Status -eq 'INVALID')

$rep2 = ('{"mode":"quick","error":{"message":"boom"},"stopped":null}' | ConvertFrom-Json)
$v = Get-QuickCheckVerdict -Report $rep2 -Capability (New-Cap) -Cells $cells
Check 'an app error is INVALID (the run is repeated)' ($v.Status -eq 'INVALID' -and $v.Headline -match 'boom')

$v = Get-QuickCheckVerdict -Report $report -Capability (New-Cap) -Cells @()
Check 'missing per-cell times are said plainly and do not fail the verdict' ($v.Status -eq 'PASS' -and (($v.Lines -join "`n") -match 'no engine cell was seen'))

# Quick against full: the same rolling cells, flagged above 10 percent, deciding cells marked.
function New-MCap([double]$HiTurbo, [double]$EnSmall, [double]$Excerpt) {
  ('{"check":{"excerpt_seconds":' + $Excerpt + '},"decision":{"languages":{"hi":{"rolling":{"model":"turbo"}},"en":{"rolling":{"model":"small"}}}},"measurements":[' +
    '{"model":"turbo","lang":"hi","mode":"rolling","config":{"kind":"gpu","index":0},"rtf":' + $HiTurbo + '},' +
    '{"model":"small","lang":"en","mode":"rolling","config":{"kind":"cpu"},"rtf":' + $EnSmall + '},' +
    '{"model":"small","lang":"en","mode":"near_live","config":{"kind":"cpu"},"rtf":0.2}]}') | ConvertFrom-Json
}
$cmp = Get-QuickFullComparison -Quick (New-MCap 0.80 0.30 30) -Full (New-MCap 0.70 0.31 60)
Check 'two rolling cells are compared; the near-live cell is not' ($cmp.Compared -eq 2)
Check 'a 14 percent difference on the first-choice Hindi cell is flagged and DECIDING' ($cmp.Flagged -eq 1 -and $cmp.DecidingFlagged -eq 1 -and (($cmp.Lines -join "`n") -match 'FLAG DECIDING'))
Check 'a 3 percent difference is not flagged' (@($cmp.Rows | Where-Object { $_.Cell -match 'small' -and $_.Flag }).Count -eq 0)
$cmp = Get-QuickFullComparison -Quick $null -Full (New-MCap 0.7 0.3 60)
Check 'a missing run is said plainly' ($cmp.Compared -eq 0 -and $cmp.Lines[0] -match 'not compared')

if ($fails) { Write-Host "$fails failed"; exit 1 }
Write-Host 'QuickCheck tests passed'
