# Gate network-off: with the network adapters off,
#   1. every engine of the pack runs `info` and a short decode (avx2, avx512 where the CPU has it,
#      vulkan on the CPU and each graphics adapter) and no non-loopback socket is seen in its process
#      tree (scripts\gates\network-off.ps1);
#   2. the release app starts three times and renders, with no non-loopback socket in its process tree
#      or its WebView2 processes (scripts\offline-check.ps1);
#   3. a static check: no shipped executable imports Winsock, WinHTTP or WinINet. A positive control
#      (a Windows program that does) proves the reader works.
# The person switches the network off for this gate and back on at the end; this gate asks.
# Windows PowerShell 5.1, ASCII only.
param([string]$CtxFile, [string]$ResultFile)
$ErrorActionPreference = 'Stop'
$lib = Join-Path $PSScriptRoot '..\..\lib'
Import-Module (Join-Path $lib 'PackCore.psm1') -Force
Import-Module (Join-Path $lib 'PackGate.psm1') -Force
$ctx = Read-PackCtx -CtxFile $CtxFile
Initialize-PackGate -Ctx $ctx
Import-Module (Join-Path $lib 'GateRun.psm1') -Force
Import-Module (Join-Path $lib 'Redact.psm1') -Force
Import-Module (Join-Path $lib 'NetworkState.psm1') -Force
Import-Module (Join-Path $lib 'PeImports.psm1') -Force
$paths = Get-PackPaths -Ctx $ctx

function Get-NetState {
  $facts = Get-NetworkFacts
  $r = Resolve-NetworkState -Facts $facts
  if ($r.State -ne 'Online') { $r = Resolve-NetworkState -Facts $facts -ProbeResults @(Test-InternetProbe) }
  [pscustomobject]@{ State = [string]$r.State; Lines = @(Format-NetworkState -Facts $facts -Result $r) }
}

Invoke-PackGateBody -Ctx $ctx -Id 'network-off' -ResultFile $ResultFile -Body {
  $quick = [bool]$ctx.quick
  $gdir = [string]$ctx.gate_dir
  $evDir = Join-Path $gdir 'evidence'
  New-Item -ItemType Directory -Force -Path $evDir | Out-Null
  $fpPath = Join-Path ([string]$ctx.work_dir) 'fingerprint.json'
  $fp = Read-PackJson -Path $fpPath
  $avx512 = [bool](Get-PackProp $fp 'avx512f' $false)
  $notes = New-Object System.Collections.Generic.List[string]
  $problems = New-Object System.Collections.Generic.List[string]

  # ---- 3. The static check first: it needs no network change ----
  $triple = 'x86_64-pc-windows-msvc'
  $files = @()
  foreach ($v in @('avx2', 'avx512', 'vulkan')) { $f = Join-Path $paths.Engines "wb-engine-$v-$triple.exe"; if (Test-Path -LiteralPath $f) { $files += $f } }
  if (Test-Path -LiteralPath $paths.ReleaseExe) { $files += $paths.ReleaseExe }
  $static = Test-PackNetworkImports -Files $files
  $staticLines = @('static check: imports of every shipped executable (no development tool; a pure PowerShell reader of the PE import tables)')
  $staticLines += ('  positive control: {0}' -f $(if ($static.ControlOk) { "{0} imports {1}: the reader sees network libraries where they exist" -f (Split-Path $static.Control.Path -Leaf), ($static.Control.Dlls -join ', ') } else { 'NOT FOUND: the reader could not be shown to work, so the check proves nothing' }))
  foreach ($r in $static.Results) { $staticLines += ('  {0}: {1} libraries imported; network libraries: {2}{3}' -f (Split-Path $r.Path -Leaf), @($r.Imports).Count, $(if (@($r.Network).Count) { $r.Network -join ', ' } else { 'none' }), $(if ($r.Error) { "; UNREADABLE: $($r.Error)" } else { '' })) }
  if (-not $static.ControlOk) { $problems.Add('static check: the positive control was not found, the reader is not proven') }
  foreach ($f in $static.Findings) { $problems.Add(("static check: {0} imports {1}" -f (Split-Path $f.Path -Leaf), ($f.Network -join ', '))) }
  foreach ($u in $static.Unreadable) { $problems.Add(("static check: {0} could not be read ({1})" -f (Split-Path $u.Path -Leaf), $u.Error)) }

  # ---- Ask the person to switch the network off ----
  $state = Get-NetState
  $tries = 0
  while ($state.State -ne 'Offline' -and $tries -lt 4) {
    $tries++
    $ans = Request-PackPrompt -Ctx $ctx -Message ("Switch the network OFF now:`r`n  - turn Wi-Fi off (the Wi-Fi key, the airplane-mode switch, or Settings > Network)`r`n  - unplug any network cable, disconnect any VPN or phone hotspot`r`nThe pack watches the engines and the app for any network use while it is off.`r`nWhen the network is off, press Enter. (Type SKIP to do this block later: nothing else is lost.)")
    if ($ans -match '^(?i)skip') {
      return (New-PackResult -Id 'network-off' -Status 'PENDING' -Headline 'skipped by the person; run it later with: run-gates.ps1 -Only network-off' -Quick $quick)
    }
    $state = Get-NetState
  }
  if ($state.State -ne 'Offline') {
    return (New-PackResult -Id 'network-off' -Status 'PENDING' -Headline ("the network is still $($state.State); switch it off and run: run-gates.ps1 -Only network-off") -Notes $state.Lines -Quick $quick)
  }

  $engineOut = Join-Path $evDir 'network-off-engines.txt'
  $clip = Join-Path $paths.Data 'net-clip-en.wav'
  $clipSeconds = 0.0
  $clipMeta = Join-Path $paths.Data 'net-clip-en.json'
  if (Test-Path -LiteralPath $clipMeta) { $clipSeconds = [double](Read-PackJson -Path $clipMeta).duration_s }
  if (-not (Test-Path -LiteralPath $clip)) { $clip = Join-Path $paths.BenchClips 'bench-en.wav'; $clipSeconds = 60 }
  $variants = $(if ($quick) { 'avx2,vulkan' } else { 'avx2,avx512,vulkan' })
  $args1 = @('-Profiles', 'native', '-Variants', $variants, '-Root', (ConvertTo-PackArg (Join-Path ([string]$ctx.work_dir) 'engines-netoff')), '-Out', (ConvertTo-PackArg $engineOut),
    '-Machine', (ConvertTo-PackArg ([string]$ctx.machine)), '-Clip', (ConvertTo-PackArg $clip), '-ClipSeconds', "$clipSeconds")
  if ($avx512) { $args1 += '-Avx512Present' }
  $e = Invoke-PackLegacyGate -Ctx $ctx -Script 'network-off.ps1' -Arguments $args1 -EvidenceName 'x' -TimeoutMinutes 60
  $engineVerdict = ''
  if (Test-Path -LiteralPath $engineOut) { $engineVerdict = (@(Get-Content -LiteralPath $engineOut -Encoding UTF8 | Where-Object { $_ -match '^NETWORK-OFF RESULT' }) | Select-Object -Last 1) }
  if ($e.ExitCode -ne 0 -or $engineVerdict -notmatch 'PASS') {
    $problems.Add("engines with the network off: exit $($e.ExitCode); $engineVerdict")
  }

  # ---- 2. The release app ----
  $appLines = @()
  if (Test-Path -LiteralPath $paths.ReleaseExe) {
    $oc = Join-Path ([string]$ctx.pack_root) 'scripts\offline-check.ps1'
    $ocOut = Join-Path $evDir 'offline-app'
    New-Item -ItemType Directory -Force -Path $ocOut | Out-Null
    $ocArgs = @('-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', (ConvertTo-PackArg $oc), '-Label', 'offline', '-Exe', (ConvertTo-PackArg $paths.ReleaseExe), '-OutDir', (ConvertTo-PackArg $ocOut), '-NoPrompt', '-Runs', $(if ($quick) { '1' } else { '3' }), '-Machine', (ConvertTo-PackArg ([string]$ctx.machine)))
    $psExe = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
    $p = Start-Process -FilePath $psExe -ArgumentList $ocArgs -PassThru -WindowStyle Hidden -RedirectStandardOutput (Join-Path $gdir 'offline-check.out.log') -RedirectStandardError (Join-Path $gdir 'offline-check.err.log')
    $null = $p.Handle
    if (-not $p.WaitForExit(20 * 60000)) { & taskkill.exe /F /T /PID $p.Id | Out-Null; $problems.Add('release app check: did not finish in 20 minutes') }
    else {
      $p.WaitForExit()
      $appLines += (@(Get-Content -LiteralPath (Join-Path $gdir 'offline-check.out.log') -ErrorAction SilentlyContinue) | Where-Object { $_ -match 'Result:|run \d|non-loopback|render' } | Select-Object -Last 8)
      if ($p.ExitCode -ne 0) { $problems.Add("release app with the network off: offline-check exit $($p.ExitCode) (1 = a non-loopback socket or no render)") }
    }
  } else { $notes.Add('the release app is not in this pack: the app part of this gate did not run'); $problems.Add('release app is not in the pack') }

  # ---- Switch the network back on ----
  $stateEnd = Get-NetState
  $lines = @('Network-off gate (gate pack)', ("date: {0}" -f (Get-Date).ToString('yyyy-MM-dd HH:mm', [Globalization.CultureInfo]::InvariantCulture)), 'network state at the start:')
  $lines += @($state.Lines | ForEach-Object { "  $_" })
  $lines += ''
  $lines += $staticLines
  $lines += ''
  $lines += 'engines with the network off (details in network-off-engines.txt): ' + $engineVerdict
  $lines += 'release app (details in offline-app\): ' + (($appLines | Select-Object -Last 3) -join ' | ')
  $lines += ''
  $lines += 'network state at the end:'
  $lines += @($stateEnd.Lines | ForEach-Object { "  $_" })
  if ($stateEnd.State -ne 'Offline') { $problems.Add("the network was $($stateEnd.State) at the end: the run is not valid as an offline run") }
  $evidence = Join-Path $gdir 'network-off.txt'
  $lines += $(if ($problems.Count -eq 0) { 'NETWORK-OFF RESULT: PASS' } else { 'NETWORK-OFF RESULT: FAIL' })
  foreach ($pr in $problems) { $lines += "  problem: $pr" }
  Write-WbEvidence -Path $evidence -Lines $lines -MachineLabel ([string]$ctx.machine)

  if (-not $ctx.unattended) {
    $null = Request-PackPrompt -Ctx $ctx -Message "The network block is finished.`r`nSwitch Wi-Fi back ON now (and plug the network cable back in), then press Enter."
  }
  $status = $(if ($problems.Count -eq 0) { 'PASS' } else { 'FAIL' })
  $head = $(if ($problems.Count -eq 0) { 'engines and the release app made no network connection with the network off; no shipped executable imports Winsock, WinHTTP or WinINet' } else { ($problems -join '; ') })
  New-PackResult -Id 'network-off' -Status $status -Headline $head -Measured ([ordered]@{ static_control = $static.ControlOk; static_findings = @($static.Findings | ForEach-Object { [ordered]@{ file = (Split-Path $_.Path -Leaf); dlls = $_.Network } }); engines = $engineVerdict }) -Threshold 'no non-loopback socket in any engine or app process tree with the network off; no Winsock, WinHTTP or WinINet import in a shipped executable' -Evidence @($evidence, $engineOut) -Notes @($notes) -Quick $quick
}
