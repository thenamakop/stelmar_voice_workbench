<#
.SYNOPSIS
  Phase 2 item 12: the engines run with the network off. Run it yourself, from the repository folder,
  with Wi-Fi off and Ethernet unplugged; it refuses to run while any adapter is up.

    powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\gates\network-off.ps1

.DESCRIPTION
  1. Checks the network state with scripts/lib/NetworkState.psm1 (physical adapters, connection
     profiles and, when those say offline, a 2-second TCP probe to 1.1.1.1:443 and 8.8.8.8:443). Anything
     but 'Offline' stops the script with exit code 2 before it touches an engine.
  2. For each built engine (avx2, avx512, vulkan, cuda), from an engines-only folder with a minimal PATH
     (no development tools), under the emulation profile emu-low and natively:
       - `info` (expected exit 0; avx512 exits 3 on this CPU, as intended);
       - a short batch decode (a FLEURS English clip of 6 to 12 s, model small, 4 threads under emu-low,
         8 natively) on the CPU, and for vulkan and cuda also on every GPU device the run's `info` lists (natively the integrated GPU and the RTX; cuda is a development build, not shipped).
  3. While every engine process runs, it polls the sockets of the engine's whole process tree back to back (TCP
     connections, listeners, UDP endpoints; the evidence prints the measured mean interval) and records any that
     is not loopback; loopback ones are only counted. A listener or endpoint bound to 0.0.0.0 or :: is NOT
     loopback. A socket created more than 2 s before the engine started is listed with its owner, not counted.
  4. Checks the network state again at the end and writes docs\evidence\phase-2\network-off.txt.

  It never changes adapters or firewall rules, needs no administrator rights, writes no model or audio into
  the repository and takes about 10 to 15 minutes. "Done" is the line  NETWORK-OFF RESULT: PASS  (or FAIL
  with the reasons) at the end of the console output and of the file; then switch the network back on.
#>
[CmdletBinding()]
param(
  [string[]]$Variants = @('avx2', 'avx512', 'vulkan', 'cuda'),
  [string[]]$Profiles = @('emu-low', 'native'),
  [string]$Root = 'D:\STELMAR-Gate\engines-only-netoff',
  [string]$Out,
  [string]$Emulate,
  [string]$Model = 'small',
  [string]$Machine = 'Legion',
  [string]$FactsFile,   # test only: network facts from this JSON file instead of the machine
  [switch]$DryRun   # development only: skips the refusal and writes a file that is marked NOT EVIDENCE
)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo = (Resolve-Path (Join-Path $here '..\..')).Path
if (-not $Out) { $Out = $(if ($DryRun) { Join-Path $env:TEMP 'network-off-dryrun.txt' } else { Join-Path $repo 'docs\evidence\phase-2\network-off.txt' }) }
if (-not $Emulate) { $Emulate = Join-Path $repo 'target\release\wb-emulate.exe' }
Import-Module (Join-Path $repo 'scripts\lib\PsEnv.psm1') -Force
Set-WbCleanPSModulePath
Set-WbInvariantCulture
Import-Module (Join-Path $repo 'scripts\lib\NetworkState.psm1') -Force
Import-Module (Join-Path $repo 'scripts\lib\Redact.psm1') -Force
Import-Module (Join-Path $repo 'scripts\lib\ProcessTree.psm1') -Force
$inv = [Globalization.CultureInfo]::InvariantCulture

# ---- 1. The network must be off -------------------------------------------------------------
function Get-CheckedNetworkState {
  $facts = $(if ($FactsFile) { Get-Content -Raw -LiteralPath $FactsFile | ConvertFrom-Json } else { Get-NetworkFacts })
  $result = Resolve-NetworkState -Facts $facts
  if ($result.State -ne 'Online') {
    $probes = @(Test-InternetProbe)
    $result = Resolve-NetworkState -Facts $facts -ProbeResults $probes
  }
  [pscustomobject]@{ Facts = $facts; Result = $result; Lines = @(Format-NetworkState -Facts $facts -Result $result) }
}
$netStart = Get-CheckedNetworkState
if ($netStart.Result.State -ne 'Offline' -and -not $DryRun) {
  Write-Host ''
  Write-Host "REFUSED: the network is not off (state: $($netStart.Result.State))." -ForegroundColor Red
  foreach ($l in $netStart.Lines) { Write-Host "  $l" }
  Write-Host ''
  Write-Host 'Switch Wi-Fi off, unplug Ethernet (and disconnect any VPN or hotspot), check that every adapter is down, then run this again.'
  exit 2
}

# ---- 2. What to run ---------------------------------------------------------------------------
$triple = 'x86_64-pc-windows-msvc'
$sys32 = Join-Path $env:SystemRoot 'System32'
$minPath = "$sys32;$env:SystemRoot"
$manifest = Join-Path $repo 'models\manifest.json'
$modelsDir = Join-Path $repo 'models'
if (-not (Test-Path (Join-Path $repo 'eval\data\fleurs-en\manifest.jsonl'))) { throw 'eval\data\fleurs-en is missing: the test clip comes from there (run this on the development machine)' }
if (-not (Test-Path $Emulate)) { throw "wb-emulate not built: $Emulate (cargo build --release -p wb-emulate)" }
$clipRec = Get-Content (Join-Path $repo 'eval\data\fleurs-en\manifest.jsonl') -Encoding UTF8 |
  ForEach-Object { $_ | ConvertFrom-Json } | Where-Object { $_.duration_s -ge 6 -and $_.duration_s -le 12 } | Sort-Object id | Select-Object -First 1
$clip = Join-Path $repo ('eval\data\fleurs-en\' + ($clipRec.audio -replace '/', '\'))
if (-not (Test-Path $clip)) { throw "test clip not found: $clip" }

# Runs a command and watches its process tree for sockets. Returns exit code, output and the sockets.
function Invoke-Watched([string]$File, [string[]]$ArgumentList, [string]$Tag) {
  $o = Join-Path $env:TEMP "wb-netoff-$Tag-$PID.out"
  $e = Join-Path $env:TEMP "wb-netoff-$Tag-$PID.err"
  $saved = $env:PATH
  $env:PATH = $minPath
  try {
    $p = Start-Process -FilePath $File -ArgumentList $ArgumentList -PassThru -WindowStyle Hidden -RedirectStandardOutput $o -RedirectStandardError $e
  } finally { $env:PATH = $saved }
  $null = $p.Handle   # keeps the process handle open so ExitCode is readable after it exits
  $nonLoop = @{}; $excluded = @{}; $loopCount = 0; $polls = 0
  $rootStart = $null; try { $rootStart = [datetime]$p.StartTime } catch { }
  $rootText = "engine tree root pid $($p.Id), started $(if ($rootStart) { $rootStart.ToString('HH:mm:ss.fff', $inv) } else { 'unknown' })"
  $watch = [Diagnostics.Stopwatch]::StartNew()
  # No sleep between polls: each poll (process snapshot, two tree builds, the TCP and UDP tables) already takes a few
  # hundred ms, and the evidence prints the measured mean interval.
  while (-not $p.HasExited -and $watch.Elapsed.TotalSeconds -lt 900) {
    # One snapshot per poll; the tree is built from it with the creation-time rule (no parent-PID reuse), and the
    # owner of every flagged socket is described from the same snapshot.
    $snap = @(Get-WbProcessSnapshot)
    $ids = @(Get-WbTreeIds -Processes $snap -RootPid $p.Id)
    $polls++
    # Information only: sockets of processes the old rule (parent id alone) would have put in the tree and the
    # creation-time rule leaves out. They are listed with their owners and do not count.
    $oldIds = @(Get-WbTreeIds -Processes $snap -RootPid $p.Id -IgnoreCreationTime)
    $extra = @($oldIds | Where-Object { $ids -notcontains $_ })
    $tcp = @(Get-NetTCPConnection -ErrorAction SilentlyContinue)
    $udp = @(Get-NetUDPEndpoint -ErrorAction SilentlyContinue)
    foreach ($c in @($tcp | Where-Object { $extra -contains $_.OwningProcess })) {
      $v = Get-WbSocketVerdict -Kind TCP -State "$($c.State)" -LocalAddress $c.LocalAddress -LocalPort $c.LocalPort -RemoteAddress $c.RemoteAddress -RemotePort $c.RemotePort
      if ($v.Verdict -ne 'loopback' -and -not $excluded.ContainsKey($v.Key)) { $excluded[$v.Key] = "$($v.Key) ; $(Get-WbOwnerDescription -Processes $snap -OwnerPid ([int]$c.OwningProcess)) ; $rootText; left out: created before its parent (the parent id was reused)" }
    }
    $mine = @(@($tcp | Where-Object { $ids -contains $_.OwningProcess } | ForEach-Object { @{ Kind = 'TCP'; S = $_ } }) +
              @($udp | Where-Object { $ids -contains $_.OwningProcess } | ForEach-Object { @{ Kind = 'UDP'; S = $_ } }))
    foreach ($m in $mine) {
      $s = $m.S
      $v = Get-WbSocketVerdict -Kind $m.Kind -State "$($s.State)" -LocalAddress $s.LocalAddress -LocalPort $s.LocalPort -RemoteAddress $s.RemoteAddress -RemotePort $s.RemotePort -CreationTime $s.CreationTime -RootStart $rootStart
      if ($v.Verdict -eq 'loopback') { $loopCount++; continue }
      $d = "$($v.Key) ; $(Get-WbOwnerDescription -Processes $snap -OwnerPid ([int]$s.OwningProcess)) ; socket created $(if ($s.CreationTime) { ([datetime]$s.CreationTime).ToString('HH:mm:ss.fff', $inv) } else { 'unknown' }); $rootText"
      if ($v.Verdict -eq 'predates') {
        if (-not $excluded.ContainsKey($v.Key)) { $excluded[$v.Key] = "$d; left out: the socket was created before the engine started" }
      } elseif (-not $nonLoop.ContainsKey($v.Key)) { $nonLoop[$v.Key] = $d }
    }
  }
  if (-not $p.HasExited) { try { $p.Kill() } catch { } }
  $p.WaitForExit()
  $seconds = [math]::Round($watch.Elapsed.TotalSeconds, 1)
  $meanPollMs = $(if ($polls) { [int]($watch.Elapsed.TotalMilliseconds / $polls) } else { 0 })
  $stdout = @(Get-Content $o -Encoding UTF8 -ErrorAction SilentlyContinue)
  $stderr = @(Get-Content $e -Encoding UTF8 -ErrorAction SilentlyContinue)
  Remove-Item $o, $e -Force -ErrorAction SilentlyContinue
  [pscustomobject]@{ Exit = $p.ExitCode; Out = $stdout; Err = $stderr; NonLoopback = @($nonLoop.Keys | Sort-Object | ForEach-Object { $nonLoop[$_] }); Excluded = @($excluded.Keys | Sort-Object | ForEach-Object { $excluded[$_] }); LoopbackSamples = $loopCount; Polls = $polls; Seconds = $seconds; MeanPollMs = $meanPollMs }
}

function Get-Events($Lines) { @($Lines | Where-Object { $_ -match '^\{' } | ForEach-Object { try { $_ | ConvertFrom-Json } catch { $null } } | Where-Object { $_ }) }

$lines = @(
  'Engines run with the network off (Phase 2 item 12)',
  "date: $((Get-Date).ToString('yyyy-MM-dd HH:mm', $inv)) (IST on the Legion)",
  "machine: $Machine; PATH while running: $minPath (no development tools); test clip: a FLEURS English clip of $([math]::Round([double]$clipRec.duration_s, 1).ToString($inv)) s (no audio or text below); model $Model; models in the repository's models folder",
  '',
  'network state at the start (scripts/lib/NetworkState.psm1):'
) + @($netStart.Lines | ForEach-Object { "  $_" }) + @('')

$problems = @()
$rows = @()
$totalNonLoop = 0
foreach ($v in $Variants) {
  $src = Join-Path $repo "src-tauri\binaries\wb-engine-$v-$triple.exe"
  if (-not (Test-Path $src)) { $lines += "$v : engine not built ($src missing)"; $problems += "$v : engine not built"; continue }
  $dir = Join-Path $Root $v
  if (Test-Path $dir) { Remove-Item $dir -Recurse -Force }
  New-Item -ItemType Directory -Force -Path $dir | Out-Null
  $exe = Join-Path $dir 'wb-engine.exe'
  Copy-Item $src $exe
  # The CUDA runtime DLLs, copied beside the engine for this test only (as engines-only.ps1 does).
  if ($v -eq 'cuda' -and $env:CUDA_PATH) {
    foreach ($b in @((Join-Path $env:CUDA_PATH 'bin'), (Join-Path $env:CUDA_PATH 'bin\x64'))) {
      if (Test-Path $b) { Get-ChildItem $b -Filter '*.dll' -ErrorAction SilentlyContinue | Where-Object { $_.Name -match '^(cudart|cublas|cublasLt)' } | ForEach-Object { Copy-Item $_.FullName $dir -Force } }
    }
  }
  $lines += "== $v"
  foreach ($p in $Profiles) {
    $threads = $(if ($p -eq 'emu-low') { 4 } else { 8 })
    # info
    $argsInfo = $(if ($p -eq 'native') { @('info') } else { @('--profile', $p, '--', ('"' + $exe + '"'), 'info') })
    $fileInfo = $(if ($p -eq 'native') { $exe } else { $Emulate })
    $r = Invoke-Watched -File $fileInfo -ArgumentList $argsInfo -Tag "$v-$p-info"
    $ev = Get-Events $r.Out
    $info = $ev | Where-Object { $_.event -eq 'info' } | Select-Object -First 1
    $expect = $(if ($v -eq 'avx512') { 3 } else { 0 })
    $okInfo = ($r.Exit -eq $expect)
    if (-not $okInfo) { $problems += "$v/$p info: exit $($r.Exit), expected $expect" }
    $totalNonLoop += $r.NonLoopback.Count
    if ($r.NonLoopback.Count) { $problems += "$v/$p info: non-loopback sockets: $($r.NonLoopback -join '; ')" }
    $lines += "-- $p info : exit code $($r.Exit) (expected $expect)$(if ($okInfo) { ' ok' } else { ' WRONG' }); $($r.Seconds) s; sockets of the process tree: non-loopback $($r.NonLoopback.Count), loopback samples $($r.LoopbackSamples), polls $($r.Polls) (mean $($r.MeanPollMs) ms apart)"
    foreach ($s in $r.NonLoopback) { $lines += "   NON-LOOPBACK SOCKET: $s" }
    foreach ($s in $r.Excluded) { $lines += "   not in the engine tree (not counted): $s" }
    $rows += [pscustomobject]@{ Variant = $v; Profile = $p; Step = 'info'; Exit = $r.Exit; Ok = $okInfo; NonLoop = $r.NonLoopback.Count }

    # Devices to decode on: the CPU always (except avx512 on this CPU), and for vulkan and cuda the first GPU device.
    if ($v -eq 'avx512') {
      $lines += "-- $p batch : not run (this CPU has no AVX-512: info exits 3 by design; the avx512 decode is a laptop gate)"
      continue
    }
    $targets = @(@{ Name = 'cpu'; Arg = 'cpu' })
    if ($info -and $v -in @('vulkan', 'cuda')) {
      $want = $(if ($v -eq 'vulkan') { 'Vulkan' } else { 'CUDA' })
      # Every GPU the run's info lists for this backend (natively: the integrated GPU and the RTX).
      $gpus = @($info.devices | Where-Object { $_.backend -eq $want -and $_.kind -ne 'cpu' })
      foreach ($gpu in $gpus) { $targets += @{ Name = "gpu:$($gpu.index) ($($gpu.description))"; Arg = "gpu:$($gpu.index)" } }
      if (-not $gpus.Count) { $lines += "   no $want device listed under $p : the GPU decode is skipped" }
    }
    foreach ($t in $targets) {
      $outFile = Join-Path $Root "batch-$v-$p-$($t.Arg -replace '[^a-z0-9]', '_').jsonl"
      $batchArgs = @('batch', '--file', ('"' + $clip + '"'), '--models', "en=$Model", '--langs', 'en', '--out', ('"' + $outFile + '"'), '--threads', "$threads", '--device', $t.Arg, '--models-dir', ('"' + $modelsDir + '"'))
      $argsB = $(if ($p -eq 'native') { $batchArgs } else { @('--profile', $p, '--', ('"' + $exe + '"')) + $batchArgs })
      $fileB = $(if ($p -eq 'native') { $exe } else { $Emulate })
      $rb = Invoke-Watched -File $fileB -ArgumentList $argsB -Tag "$v-$p-batch"
      $evb = Get-Events $rb.Out
      $segs = @($evb | Where-Object { $_.event -eq 'segment' })
      $done = $evb | Where-Object { $_.event -eq 'done' } | Select-Object -First 1
      $errB = $evb | Where-Object { $_.event -eq 'error' } | Select-Object -First 1
      $letters = 0; foreach ($s in $segs) { $letters += ([regex]::Matches([string]$s.text, '[A-Za-z]')).Count }
      $okB = ($rb.Exit -eq 0 -and $segs.Count -gt 0 -and $letters -gt 20 -and $done)
      if (-not $okB) { $problems += "$v/$p batch on $($t.Name): exit $($rb.Exit), segments $($segs.Count)$(if ($errB) { ', error ' + $errB.code })" }
      $totalNonLoop += $rb.NonLoopback.Count
      if ($rb.NonLoopback.Count) { $problems += "$v/$p batch on $($t.Name): non-loopback sockets: $($rb.NonLoopback -join '; ')" }
      $lines += "-- $p batch on $($t.Name) : exit code $($rb.Exit); segments $($segs.Count), Latin letters $letters (the text is not printed); $($rb.Seconds) s; $(if ($okB) { 'ok' } else { 'WRONG' }); sockets: non-loopback $($rb.NonLoopback.Count), loopback samples $($rb.LoopbackSamples), polls $($rb.Polls) (mean $($rb.MeanPollMs) ms apart)"
      if ($errB) { $lines += "   error event: $($errB.code): $($errB.message)" }
      if (-not $okB) { foreach ($e in $rb.Err | Select-Object -Last 6) { if ("$e".Trim()) { $lines += "   stderr: $("$e".Trim())" } } }
      foreach ($s in $rb.NonLoopback) { $lines += "   NON-LOOPBACK SOCKET: $s" }
      foreach ($s in $rb.Excluded) { $lines += "   not in the engine tree (not counted): $s" }
      $rows += [pscustomobject]@{ Variant = $v; Profile = $p; Step = "batch $($t.Name)"; Exit = $rb.Exit; Ok = $okB; NonLoop = $rb.NonLoopback.Count }
      Remove-Item $outFile -Force -ErrorAction SilentlyContinue
    }
  }
  $lines += ''
}

# ---- 4. The network must still be off at the end ----------------------------------------------
$netEnd = Get-CheckedNetworkState
$lines += 'network state at the end:'
$lines += @($netEnd.Lines | ForEach-Object { "  $_" })
if ($netEnd.Result.State -ne 'Offline' -and -not $DryRun) { $problems += "the network state at the end is $($netEnd.Result.State): the run is not valid as an offline run" }
$lines += ''
$lines += ('summary: ' + (($rows | ForEach-Object { "$($_.Variant)/$($_.Profile)/$($_.Step) exit $($_.Exit) $(if ($_.Ok) { 'ok' } else { 'WRONG' })" }) -join '; '))
$lines += "socket check: non-loopback TCP connections, TCP listeners or UDP endpoints seen in the engines' process trees: $totalNonLoop in $(@($rows).Count) runs"
$lines += 'what PASS means: no standing non-loopback socket or endpoint was observed in an engine process tree at any poll (the polls are a few hundred ms apart, so a connection attempt that fails at once is not seen; the static check that no engine or app binary imports Winsock, WinHTTP or WinINet covers that). A wildcard listener (0.0.0.0 or ::) is a non-loopback socket. A socket created more than 2 s before its engine started is listed with its owner and not counted.'
if ($DryRun) {
  $lines += "DRY RUN: the network was $($netStart.Result.State); this is NOT evidence. Problems found: $($problems.Count)"
  foreach ($pr in $problems) { $lines += "  problem: $pr" }
} elseif ($problems.Count -eq 0) {
  $lines += 'NETWORK-OFF RESULT: PASS (every engine ran with the network off: info and a short decode, no non-loopback socket)'
} else {
  $lines += 'NETWORK-OFF RESULT: FAIL'
  foreach ($pr in $problems) { $lines += "  problem: $pr" }
}
Write-WbEvidence -Path $Out -Lines $lines -MachineLabel $Machine
Write-Host ''
Write-Host ($lines[-1])
Write-Host "wrote $Out"
Write-Host 'Done. You can switch the network back on now.'
if ($problems.Count) { exit 1 }
