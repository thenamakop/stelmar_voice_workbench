# Tests for scripts/lib/SpeedTable.psm1: the plan of cells for each part and the evidence lines.
# Run: powershell -NoProfile -ExecutionPolicy Bypass -File scripts/tests/SpeedTable.Tests.ps1
# ASCII only; Windows PowerShell 5.1.
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
Import-Module (Join-Path $here '..\lib\SpeedTable.psm1') -Force
$fails = 0
function Check([string]$Name, [bool]$Ok) {
  if ($Ok) { Write-Host "ok   $Name" } else { Write-Host "FAIL $Name"; $script:fails++ }
}
$all = @('small', 'medium', 'large-v3-turbo-q5_0', 'large-v3-q5_0', 'hinglish-q5_0')
$igpu = [pscustomobject]@{ Index = 1; Backend = 'Vulkan'; Description = 'Intel(R) Iris(R) Xe Graphics'; Kind = 'integrated'; Driver = '31.0.101.4502' }
$dgpu = [pscustomobject]@{ Index = 0; Backend = 'Vulkan'; Description = 'NVIDIA GeForce RTX 4070 Laptop GPU'; Kind = 'discrete'; Driver = '560.94' }
$cpuDev = [pscustomobject]@{ Index = 2; Backend = 'Vulkan'; Description = 'llvmpipe'; Kind = 'cpu'; Driver = '' }

$p = @(Get-SpeedPlan -Part igpu -Devices @($igpu, $dgpu, $cpuDev) -Present $all)
Check 'igpu: every real adapter is a row, a cpu device is not' (@($p | Select-Object -ExpandProperty Row -Unique).Count -eq 2)
Check 'igpu: the driver version is in the row name' (@($p | Where-Object { $_.Row -match '31\.0\.101\.4502' }).Count -gt 0)
Check 'igpu: 5 models x 2 languages + 1 code-switched row, per adapter' ($p.Count -eq 2 * (5 * 2 + 1))
Check 'igpu: the code-switched clip is used only for the Hinglish model' (@($p | Where-Object { $_.Clip -eq 'bench-codeswitch.wav' } | Select-Object -ExpandProperty Model | Select-Object -ExpandProperty Id -Unique) -join ',' -eq 'hinglish-q5_0')
Check 'igpu: GPU rows run on the vulkan engine with 2 CPU threads' (@($p | Where-Object { $_.Variant -ne 'vulkan' -or $_.Threads -ne 2 }).Count -eq 0)

$pq = @(Get-SpeedPlan -Part igpu -Devices @($dgpu, $igpu) -Present $all -Quick $true)
Check 'igpu quick: one model, one clip, the integrated adapter' ($pq.Count -eq 1 -and $pq[0].Model.Id -eq 'small' -and $pq[0].Row -match 'Iris')

$pm = @(Get-SpeedPlan -Part igpu -Devices @($igpu) -Present @('small', 'medium'))
Check 'a model missing from the pack is left out of the plan' (@($pm | Select-Object -ExpandProperty Model | Select-Object -ExpandProperty Id -Unique).Count -eq 2)

$c = @(Get-SpeedPlan -Part cpu -PhysicalCores 4 -LogicalProcessors 8 -HasAvx512 $false -Present $all)
Check 'cpu: avx2 at the app count (physical minus 1), the physical and the logical thread count, no avx512 without AVX-512' (@($c | Where-Object { $_.Variant -eq 'avx512' }).Count -eq 0 -and (@($c | Where-Object { $_.Variant -eq 'avx2' -and $_.Mode -eq 'rolling' } | Select-Object -ExpandProperty Threads -Unique | Sort-Object) -join ',') -eq '3,4,8')
Check 'cpu: small near-live at half the logical processors, English and Hindi' (@($c | Where-Object { $_.Mode -eq 'near_live' }).Count -eq 2 -and @($c | Where-Object { $_.Mode -eq 'near_live' -and $_.Threads -eq 4 }).Count -eq 2)
$c5 = @(Get-SpeedPlan -Part cpu -PhysicalCores 8 -LogicalProcessors 16 -HasAvx512 $true -Present $all)
Check 'cpu: avx512 rows appear only with AVX-512' (@($c5 | Where-Object { $_.Variant -eq 'avx512' }).Count -gt 0)
$c1 = @(Get-SpeedPlan -Part cpu -PhysicalCores 4 -LogicalProcessors 4 -Present $all)
Check 'cpu: equal physical and logical counts give two thread sets (the app count and the shared count)' ((@($c1 | Where-Object { $_.Mode -eq 'rolling' } | Select-Object -ExpandProperty Threads -Unique | Sort-Object) -join ',') -eq '3,4')
$c11 = @(Get-SpeedPlan -Part cpu -PhysicalCores 1 -LogicalProcessors 1 -Present $all)
Check 'cpu: a single-core machine gives one thread set' (@($c11 | Where-Object { $_.Mode -eq 'rolling' } | Select-Object -ExpandProperty Threads -Unique).Count -eq 1)
$cq = @(Get-SpeedPlan -Part cpu -PhysicalCores 4 -LogicalProcessors 8 -HasAvx512 $true -Present $all -Quick $true)
Check 'cpu quick: one model, one clip, avx2 only' ($cq.Count -eq 1 -and $cq[0].Variant -eq 'avx2' -and $cq[0].Lang -eq 'en')
Check 'keys are unique' ((@($c5 | Select-Object -ExpandProperty Key -Unique)).Count -eq $c5.Count)

$recs = @(
  [pscustomobject]@{ key = 'k1'; row = 'vulkan on X (device 0, driver 1.2), 2 CPU threads'; order = 1; model = 'small'; model_order = 0; clip = 'bench-en.wav'; mode = 'rolling'; threads = 2; variant = 'vulkan'; engine_sha = 'abc'; status = 'ok'; rtf_median = 0.42; rtf_runs = @(0.41, 0.42, 0.43); warmup_rtf = 0.5; load_s = 1.2; peak_ws = 1GB; peak_commit = 2GB; gpu_local_bytes = 512MB; gpu_shared_bytes = 0; segments = 12; truncated = 0; single_run = $false; whisper = 'v1 commit c tree t' },
  [pscustomobject]@{ key = 'k2'; row = 'vulkan on X (device 0, driver 1.2), 2 CPU threads'; order = 1; model = 'large-v3 q5_0'; model_order = 3; clip = 'bench-hi.wav'; mode = 'rolling'; threads = 2; variant = 'vulkan'; engine_sha = 'abc'; status = 'does_not_fit'; peak_ws = 5GB; peak_commit = 6GB; exit = 1; error = 'out of memory' })
$lines = @(Get-SpeedReportLines -Part igpu -Records $recs -Machine 'M1' -Profile 'laptop')
$text = $lines -join "`n"
Check 'report: row, model, RTF with runs, the adapter and driver' ($text -match 'driver 1\.2' -and $text -match 'small \| bench-en\.wav \| rolling \| 2 \| 0\.420 \(0\.410, 0\.420, 0\.430\)')
Check 'report: a cell that does not fit says so and keeps its memory' ($text -match 'does not fit' -and $text -match 'out of memory')
Check 'report: counts' ($text -match 'cells: 2 \(ok 1, does not fit 1, failed 0')
Check 'report: says it is information only' ($text -match 'information only')

if ($fails) { Write-Host "$fails failed"; exit 1 }
Write-Host 'SpeedTable tests passed'
