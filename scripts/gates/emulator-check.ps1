<#
.SYNOPSIS
  Phase 1 emulator gate: what a child process sees under emu-low and emu-mid.

.DESCRIPTION
  Runs `wb-emulate probe` inside each profile's job and checks:
    emu-low: exactly 4 logical processors, all efficiency cores (affinity mask logged);
    emu-mid: exactly 12 logical processors (2 performance cores with Hyper-Threading and 8
             efficiency cores);
    both:    commit capped at the 11 GiB job limit (at least 10.5 GiB reachable, no more than
             11 GiB: "11 GB" in docs/LEGION-FIRST.md means binary gigabytes, as a "16 GB" laptop
             has 16 GiB);
             CUDA sees no device inside the job, and a process that tries to hold a CUDA context
             there does not appear in nvidia-smi;
             a compute load on the integrated GPU ('UHD') runs inside the job.
  Positive control, outside any job: the same probe sees at least one CUDA device and, while it
  holds a context, nvidia-smi lists it. Without that, "no job process in nvidia-smi" could pass
  for the wrong reason.
  Same rules as the other gates: overlay tools closed, D:\Projects\stelmar-gate.lock held, evidence
  redacted. Needs no administrator rights.
#>
[CmdletBinding()]
param(
  [string]$Machine = 'Legion',
  [string]$GpuName = 'UHD',
  [string]$OutDir,
  [string]$Testbench,
  [string]$Emulate
)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo = Resolve-Path (Join-Path $here '..\..')
if (-not $OutDir) { $OutDir = Join-Path $repo 'docs\evidence\phase-1' }
if (-not $Testbench) { $Testbench = Join-Path $repo 'target\release\wb-testbench.exe' }
if (-not $Emulate) { $Emulate = Join-Path $repo 'target\release\wb-emulate.exe' }
Import-Module (Join-Path $repo 'scripts\lib\PsEnv.psm1') -Force
Set-WbCleanPSModulePath
Import-Module (Join-Path $repo 'scripts\lib\GateRun.psm1') -Force
Import-Module (Join-Path $repo 'scripts\lib\Redact.psm1') -Force
Import-Module (Join-Path $repo 'scripts\lib\AppProcess.psm1') -Force

function Get-NvidiaPids {
  $pids = @()
  foreach ($l in (Invoke-WbNative -FilePath 'nvidia-smi').Output) { if ($l -match '^\|\s+\S+\s+\S+\s+\S+\s+(\d+)\s+(C\+G|C|G)\s') { $pids += [int]$Matches[1] } }
  $pids
}

function Get-Value([string[]]$Text, [string]$Key) {
  (($Text | Where-Object { $_ -match "^$Key=" } | Select-Object -First 1) -replace "^$Key=", '')
}

# Starts `wb-emulate probe --cuda-hold 15` (inside a profile, or with no job for 'none'), samples
# nvidia-smi while it holds, and returns its output and whether nvidia-smi listed it.
function Invoke-CudaHold([string]$Profile) {
  $outFile = Join-Path $env:TEMP "wb-emu-cuda-$Profile.txt"
  $procArgs = $(if ($Profile -eq 'none') { @('probe', '--commit-gb', '0', '--cuda-hold', '15') } else { @('--profile', $Profile, '--', (ConvertTo-WbArg $Emulate), 'probe', '--commit-gb', '0', '--cuda-hold', '15') })
  $p = Start-Process -FilePath $Emulate -ArgumentList $procArgs -PassThru -WindowStyle Hidden -RedirectStandardOutput $outFile -RedirectStandardError (Join-Path $env:TEMP "wb-emu-cuda-$Profile.err")
  Start-Sleep -Seconds 6
  $tree = @(Get-WbProcessTree $p.Id | ForEach-Object { [int]$_.ProcessId })
  $nv = @(Get-NvidiaPids)
  $listed = @($nv | Where-Object { $tree -contains $_ })
  [void]$p.WaitForExit(60000)
  $text = @(Get-Content $outFile -ErrorAction SilentlyContinue | ForEach-Object { "$_" })
  [pscustomobject]@{ Text = $text; Devices = [int](Get-Value $text 'cuda_devices'); Held = (Get-Value $text 'cuda_context_held') -eq 'true'; Listed = $listed.Count; NvTotal = $nv.Count }
}

$GIB = [math]::Pow(2, 30)
try {
$results = Invoke-WbGate -Name 'phase1-emulator' -Body {
  $control = Invoke-CudaHold 'none'
  $out = @()
  foreach ($p in @('emu-low', 'emu-mid')) {
    $emuLog = Join-Path $env:TEMP "wb-emu-$p.log"
    $probeOut = Join-Path $env:TEMP "wb-emu-probe-$p.txt"
    $pr = Start-Process -FilePath $Emulate -ArgumentList @('--profile', $p, '--log', (ConvertTo-WbArg $emuLog), '--', (ConvertTo-WbArg $Emulate), 'probe', '--commit-gb', '16') -PassThru -WindowStyle Hidden -RedirectStandardOutput $probeOut -RedirectStandardError (Join-Path $env:TEMP "wb-emu-probe-$p.err")
    [void]$pr.WaitForExit(120000)
    $text = @(Get-Content $emuLog -ErrorAction SilentlyContinue | ForEach-Object { "wb-emulate: $_" }) + @(Get-Content $probeOut -ErrorAction SilentlyContinue | ForEach-Object { "$_" })
    $cuda = Invoke-CudaHold $p
    $stress = Start-Process -FilePath $Emulate -ArgumentList @('--profile', $p, '--', (ConvertTo-WbArg $Testbench), 'stress', '--threads', '1', '--gpu', $GpuName, '--seconds', '15') -PassThru -WindowStyle Hidden -RedirectStandardOutput (Join-Path $env:TEMP "wb-emu-stress-$p.json") -RedirectStandardError (Join-Path $env:TEMP "wb-emu-stress-$p.err")
    [void]$stress.WaitForExit(60000)
    $stressRes = Get-Content (Join-Path $env:TEMP "wb-emu-stress-$p.json") -Raw -ErrorAction SilentlyContinue | ConvertFrom-Json
    $out += [pscustomobject]@{
      Profile = $p; Probe = $text; Cuda = $cuda; Stress = $stressRes
      Logical = [int](Get-Value $text 'logical_processors')
      AllEfficiency = (Get-Value $text 'all_on_lowest_efficiency_class') -eq 'true'
      Classes = Get-Value $text 'logical_with_efficiency_class'
      CommitBytes = [double](((Get-Value $text 'max_commit_bytes') -split ' ')[0])
    }
  }
  [pscustomobject]@{ Control = $control; Profiles = $out }
}
} catch {
  $detail = $(if ($script:Current -and $script:Current.Error) { $script:Current.Error } else { ($_.Exception.GetType().FullName + ': ' + $_.Exception.Message + "`r`n" + $_.ScriptStackTrace) })
  Write-WbGateCrashEvidence -Path (Join-Path $OutDir 'emulator-check.txt') -Reason "the emulator check stopped on an error: $($_.Exception.Message)" -Detail $detail -Machine $Machine -Builds @($Emulate, $Testbench)
  Write-Host "FAIL: the emulator check did not complete: $($_.Exception.Message)"
  exit 1
}
$results = @($results)[-1]

$body = @()
$pass = $true
$c = $results.Control
$controlOk = ($c.Devices -gt 0 -and $c.Held -and $c.Listed -gt 0)
if (-not $controlOk) { $pass = $false }
$body += ''
$body += "positive control (no job): $(if ($controlOk) { 'PASS' } else { 'FAIL' })"
$body += "  [$(if ($c.Devices -gt 0) { 'x' } else { ' ' })] CUDA sees $($c.Devices) device(s) outside the job"
$body += "  [$(if ($c.Held -and $c.Listed -gt 0) { 'x' } else { ' ' })] a held CUDA context appears in nvidia-smi (listed $($c.Listed) of $($c.NvTotal))"
foreach ($r in $results.Profiles) {
  $want = $(if ($r.Profile -eq 'emu-low') { 4 } else { 12 })
  $checks = [ordered]@{}
  $checks["sees exactly $want logical processors (saw $($r.Logical))"] = ($r.Logical -eq $want)
  if ($r.Profile -eq 'emu-low') { $checks['all of them efficiency cores'] = $r.AllEfficiency }
  else { $checks['4 performance-core threads and 8 efficiency cores'] = (([regex]::Matches($r.Classes, ', 1\)')).Count -eq 4 -and ([regex]::Matches($r.Classes, ', 0\)')).Count -eq 8) }
  $checks[("commit capped by the 11 GiB job limit (reached {0:n2} GiB of 16 tried; needs 10.5 to 11)" -f ($r.CommitBytes / $GIB))] = ($r.CommitBytes -le 11 * $GIB -and $r.CommitBytes -ge 10.5 * $GIB)
  $checks["CUDA sees no device inside the job (saw $($r.Cuda.Devices))"] = ($r.Cuda.Devices -eq 0 -and -not $r.Cuda.Held)
  $checks["no job process in nvidia-smi while one tries to hold a CUDA context (listed $($r.Cuda.Listed))"] = ($r.Cuda.Listed -eq 0)
  $checks["a compute load runs on the integrated GPU inside the job ($($r.Stress.gpu_adapter), $($r.Stress.gpu_dispatches) dispatches)"] = ($r.Stress -and [int64]$r.Stress.gpu_dispatches -gt 0 -and "$($r.Stress.gpu_adapter)" -match $GpuName)
  $ok = -not ($checks.Values -contains $false)
  if (-not $ok) { $pass = $false }
  $body += ''
  $body += "$($r.Profile): $(if ($ok) { 'PASS' } else { 'FAIL' })"
  foreach ($k in $checks.Keys) { $body += "  [$(if ($checks[$k]) { 'x' } else { ' ' })] $k" }
  $body += '  probe output (child process inside the job):'
  $body += ($r.Probe | ForEach-Object { "    $_" })
  $body += '  CUDA hold probe inside the job:'
  $body += ($r.Cuda.Text | Where-Object { $_ -match '^cuda_' } | ForEach-Object { "    $_" })
}
$body += ''
$body += 'positive control probe output:'
$body += ($c.Text | Where-Object { $_ -match '^cuda_|^logical' } | ForEach-Object { "  $_" })
$lines = @(Get-WbGateHeader -Machine $Machine -Builds @($Emulate, $Testbench))
$lines += 'tool: target\release\wb-emulate.exe (development only, never bundled)'
$lines += "verdict: $(if ($pass) { 'PASS' } else { 'FAIL' })"
$lines += $body
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$evidence = Join-Path $OutDir 'emulator-check.txt'
Write-WbEvidence -Path $evidence -Lines $lines -MachineLabel $Machine
Write-Host "$(if ($pass) { 'PASS' } else { 'FAIL' }): $evidence"
if (-not $pass) { exit 1 }
