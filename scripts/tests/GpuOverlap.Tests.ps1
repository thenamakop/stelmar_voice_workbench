# Tests for scripts/lib/GpuOverlap.psm1 (the NVIDIA-GPU check of the recorder gates), with a fake nvidia-smi output.
# Run: powershell -NoProfile -ExecutionPolicy Bypass -File scripts/tests/GpuOverlap.Tests.ps1
# ASCII only; Windows PowerShell 5.1.
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$scripts = Split-Path -Parent $here
Import-Module (Join-Path $scripts 'lib\Redact.psm1') -Force
Import-Module (Join-Path $scripts 'lib\GpuOverlap.psm1') -Force
$fails = 0
function Check([string]$Name, [bool]$Ok) { if ($Ok) { Write-Host "ok   $Name" } else { Write-Host "FAIL $Name"; $script:fails++ } }

# What nvidia-smi prints on Windows (the GPU table first, then the process table).
$smi = @(
  '+-----------------------------------------------------------------------------------------+',
  '| NVIDIA-SMI 621.14                 Driver Version: 621.14         CUDA Version: 13.0     |',
  '|-----------------------------------------+------------------------+----------------------+',
  '| GPU  Name                  Driver-Model | Bus-Id          Disp.A | Volatile Uncorr. ECC |',
  '|   0  NVIDIA GeForce RTX 4070 ...  WDDM  |   00000000:01:00.0 Off |                  N/A |',
  '+-----------------------------------------------------------------------------------------+',
  '| Processes:                                                                              |',
  '|  GPU   GI   CI              PID   Type   Process name                        GPU Memory |',
  '|        ID   ID                                                               Usage      |',
  '|=========================================================================================|',
  '|    0   N/A  N/A            1234    C+G   ...ge\Application\msedge.exe                N/A      |',
  '|    0   N/A  N/A            4321    C+G   ...ft\EdgeWebView\Application\msedgewebview2.exe    N/A      |',
  '|    0   N/A  N/A            5555      G   C:\Windows\System32\dwm.exe                 N/A      |',
  '|    0   N/A  N/A            7777      C   ...ath with spaces\wb-engine-vulkan.exe     1024MiB      |',
  '+-----------------------------------------------------------------------------------------+'
)
$rows = @(ConvertFrom-WbNvidiaSmi -Lines $smi)
Check 'the process table is read, the GPU table and headers are not' ($rows.Count -eq 4 -and $rows[0].Pid -eq 1234 -and $rows[3].Pid -eq 7777)
Check 'the type (C+G, G, C) is kept' ($rows[0].Type -eq 'C+G' -and $rows[2].Type -eq 'G' -and $rows[3].Type -eq 'C')
Check 'a name with spaces is kept whole, without the memory column' ($rows[3].Name -eq '...ath with spaces\wb-engine-vulkan.exe')
Check 'no output gives no rows (nvidia-smi missing)' (@(ConvertFrom-WbNvidiaSmi -Lines @()).Count -eq 0 -and @(ConvertFrom-WbNvidiaSmi -Lines $null).Count -eq 0)

$long = 'C:\Users\tester\AppData\Local\Microsoft\EdgeWebView\Application\msedgewebview2.exe --type=gpu-process --user-data-dir=C:\Users\tester\AppData\Local\in.edu.bmu.stelmar.voiceworkbench\EBWebView --gpu-preferences=' + ('A' * 300)
$cmds = @{ 4321 = $long; 5555 = 'C:\Windows\system32\dwm.exe' }

# The job holds 4321 (a WebView2 GPU process) and 9999 (not on the GPU).
$rep = Get-WbGpuOverlap -NvProcesses $rows -JobPids @(4321, 9999, 100) -CommandLines $cmds -UserProfile 'C:\Users\tester' -UserName 'tester'
Check 'every nvidia-smi process gets a line' ($rep.Lines.Count -eq 4 -and $rep.Rows.Count -eq 4)
Check 'exactly the process of the job is marked' (@($rep.Overlap).Count -eq 1 -and $rep.Overlap[0].Pid -eq 4321)
Check 'the marked line says it is in the emulation job and names pid and file' (($rep.Lines | Where-Object { $_ -match 'pid 4321 C\+G msedgewebview2.exe \[IN THE EMULATION JOB\]' }).Count -eq 1)
Check 'a process outside the job is not marked' (($rep.Lines | Where-Object { $_ -match 'pid 5555 G dwm.exe;' }).Count -eq 1 -and ($rep.Lines | Where-Object { $_ -match 'pid 5555.*IN THE EMULATION JOB' }).Count -eq 0)
Check 'a process with no readable command line says so' (($rep.Lines | Where-Object { $_ -match 'pid 1234 .*command line not readable' }).Count -eq 1)
$cmd4321 = $rep.Rows | Where-Object { $_.Pid -eq 4321 } | ForEach-Object { $_.CommandLine }
Check 'a long command line is cut at 200 characters (and says so)' ($cmd4321.Length -eq 203 -and $cmd4321.EndsWith('...'))
Check 'the profile path in a command line is redacted' (($cmd4321 -notmatch 'tester') -and ($cmd4321 -match '%USERPROFILE%'))
Check 'the verdict text names the overlapping process, with pid, file and command line' ($rep.Text -match '^pid 4321 msedgewebview2.exe: .*--type=gpu-process' -and $rep.Text -notmatch 'dwm')
$none = Get-WbGpuOverlap -NvProcesses $rows -JobPids @(1, 2) -CommandLines $cmds -UserProfile 'C:\Users\tester' -UserName 'tester'
Check 'no process of the job on the GPU: no overlap and an empty verdict text' (@($none.Overlap).Count -eq 0 -and $none.Text -eq '' -and $none.Lines.Count -eq 4)
$two = Get-WbGpuOverlap -NvProcesses $rows -JobPids @(4321, 7777) -CommandLines $cmds -UserProfile 'C:\Users\tester' -UserName 'tester'
Check 'two overlapping processes are both named' (@($two.Overlap).Count -eq 2 -and $two.Text -match 'pid 4321' -and $two.Text -match 'pid 7777 wb-engine-vulkan.exe')
$empty = Get-WbGpuOverlap -NvProcesses @() -JobPids @(1) -CommandLines @{}
Check 'an empty process table gives an empty report, not an error' (@($empty.Rows).Count -eq 0 -and $empty.Text -eq '')

# The gate uses it: the snapshot lines are logged, and a FAIL names the process in the verdict line.
$gate = Get-Content -LiteralPath (Join-Path $scripts 'gates\recorder-gate.ps1') -Raw
Check 'recorder-gate.ps1 logs every nvidia-smi process of the snapshot' ($gate -match 'Get-WbGpuOverlap -NvProcesses' -and $gate -match 'foreach \(\$gl in \$gpuReport\.Lines\)')
Check 'recorder-gate.ps1 names the overlapping process in the verdict line of a FAIL' ($gate -match 'job process on the NVIDIA GPU: ' -and $gate -match '\$verdict\$nvFailText')
if ($fails) { Write-Host "$fails check(s) failed"; exit 1 }
Write-Host 'all passed'
