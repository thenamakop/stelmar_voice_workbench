# Gate engines-info: the machine fingerprint, then `wb-engine info` for every engine of the pack,
# run from an engines-only folder with PATH reduced to the Windows folders (no development tool).
#   avx2    exits 0.
#   avx512  exits 0 where this CPU has AVX-512 and 3 (with a clear message) where it has not.
#   vulkan  exits 0; each graphics adapter is listed once; the integrated one is named with its driver.
# Needs no audio, no lock, no administrator. Windows PowerShell 5.1, ASCII only.
param([string]$CtxFile, [string]$ResultFile)
$ErrorActionPreference = 'Stop'
$lib = Join-Path $PSScriptRoot '..\..\lib'
Import-Module (Join-Path $lib 'PackCore.psm1') -Force
Import-Module (Join-Path $lib 'PackGate.psm1') -Force
$ctx = Read-PackCtx -CtxFile $CtxFile
Initialize-PackGate -Ctx $ctx
Import-Module (Join-Path $lib 'GateRun.psm1') -Force
Import-Module (Join-Path $lib 'Redact.psm1') -Force
Import-Module (Join-Path $lib 'Fingerprint.psm1') -Force
Import-Module (Join-Path $lib 'PeImports.psm1') -Force
$paths = Get-PackPaths -Ctx $ctx

Invoke-PackGateBody -Ctx $ctx -Id 'engines-info' -ResultFile $ResultFile -Body {
  $fp = Get-PackFingerprint -WorkRoot ([string]$ctx.work_dir) -BackupRoot ([string]$ctx.backup_root)
  Write-PackJson -Path (Join-Path ([string]$ctx.work_dir) 'fingerprint.json') -Value $fp
  $lines = @('Engines info check on this laptop (gate pack)', ("date: {0}" -f (Get-Date).ToString('yyyy-MM-dd HH:mm', [Globalization.CultureInfo]::InvariantCulture)), '')
  $lines += 'machine:'
  $lines += Get-PackFingerprintLines -Fp $fp
  $lines += ''
  $sys32 = Join-Path $env:SystemRoot 'System32'
  $minPath = "$sys32;$env:SystemRoot"
  $devTools = @('cargo', 'cmake', 'python', 'git', 'node', 'dumpbin', 'nvcc') | Where-Object { Get-Command $_ -ErrorAction SilentlyContinue }
  $lines += ('development tools on the normal PATH (report only; the engines run with PATH = {0}): {1}' -f $minPath, $(if (@($devTools).Count) { @($devTools) -join ', ' } else { 'none' }))
  $lines += ''
  $triple = 'x86_64-pc-windows-msvc'
  $failures = New-Object System.Collections.Generic.List[string]
  $measured = [ordered]@{}
  $root = Join-Path ([string]$ctx.work_dir) 'engines-only'
  $cpuFeatures = @()
  $avx512Message = ''
  $variants = @('avx2', 'avx512', 'vulkan')
  if ([bool]$ctx.quick) { $variants = @('avx2', 'avx512', 'vulkan') }
  foreach ($v in $variants) {
    $src = Join-Path $paths.Engines "wb-engine-$v-$triple.exe"
    if (-not (Test-Path -LiteralPath $src)) { $lines += "== $v : engine is not in this pack ($src)"; $failures.Add("$v engine missing from the pack"); continue }
    $dir = Join-Path $root $v
    if (Test-Path -LiteralPath $dir) { Remove-Item -LiteralPath $dir -Recurse -Force }
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    $exe = Join-Path $dir 'wb-engine.exe'
    Copy-Item -LiteralPath $src -Destination $exe
    # What the engine imports that Windows itself does not provide.
    $imports = @(Get-PePeImports -Path $exe | ForEach-Object { $_.Dll } | Select-Object -Unique)
    $notSystem = @($imports | Where-Object { $_ -notmatch '^(api-ms-|ext-ms-)' -and -not (Test-Path -LiteralPath (Join-Path $sys32 $_)) })
    $lines += ('== {0}: {1:n1} MB; imports {2} libraries; not found in System32: {3}' -f $v, ((Get-Item -LiteralPath $exe).Length / 1MB), $imports.Count, $(if ($notSystem.Count) { $notSystem -join ', ' } else { 'none' }))
    $saved = $env:PATH
    $env:PATH = $minPath
    try { $res = Invoke-WbNative -FilePath $exe -ArgumentList @('info') } finally { $env:PATH = $saved }
    $events = @($res.Output | Where-Object { $_ -match '^\{' } | ForEach-Object { try { $_ | ConvertFrom-Json } catch { $null } } | Where-Object { $_ })
    $hello = $events | Where-Object { $_.event -eq 'hello' } | Select-Object -First 1
    $info = $events | Where-Object { $_.event -eq 'info' } | Select-Object -First 1
    $err = $events | Where-Object { $_.event -eq 'error' } | Select-Object -First 1
    $lines += "-- exit code $($res.ExitCode)"
    if ($hello) { $lines += "   whisper.cpp printed at runtime: $($hello.whisper_cpp.version) commit $($hello.whisper_cpp.commit) tree $($hello.whisper_cpp.tree_sha256)" }
    if ($err) { $lines += "   error event: $($err.code): $($err.message)" }
    foreach ($e in $res.Error) { if ("$e".Trim()) { $lines += "   stderr: $("$e".Trim())" } }
    if ($info -and $info.PSObject.Properties['cpu'] -and $info.cpu -and $info.cpu.PSObject.Properties['features']) { $cpuFeatures = @($info.cpu.features | ForEach-Object { [string]$_ }) }
    if ($v -eq 'avx512' -and $err) { $avx512Message = [string]$err.message }
    $devs = @()
    if ($info) {
      $lines += "   backends: $($info.backends -join ', '); logical processors allowed $($info.logical_processors_allowed); available RAM $([math]::Round($info.available_ram_bytes / 1GB, 1).ToString([Globalization.CultureInfo]::InvariantCulture)) GiB; raw device entries $($info.raw_device_count)"
      foreach ($d in $info.devices) {
        $mem = [math]::Round($d.total_memory_bytes / 1GB, 1).ToString([Globalization.CultureInfo]::InvariantCulture)
        $lines += "   device $($d.index): $($d.backend) / $($d.description) / $($d.kind) / $mem GiB / driver $($d.driver_version) / duplicates dropped $($d.duplicates_dropped)"
        $devs += [pscustomobject]@{ Backend = [string]$d.backend; Description = [string]$d.description; Kind = [string]$d.kind; Driver = [string]$d.driver_version }
      }
      foreach ($n in $info.notes) { $lines += "   note: $n" }
    }
    $measured[$v] = [ordered]@{ exit = $res.ExitCode; error_code = $(if ($err) { [string]$err.code } else { '' }); devices = @($devs | ForEach-Object { [ordered]@{ backend = $_.Backend; description = $_.Description; kind = $_.Kind; driver = $_.Driver } }); imports_not_in_system32 = $notSystem }
    # Verdicts.
    if ($notSystem.Count -gt 0 -and $v -ne 'vulkan') { $failures.Add("$v needs libraries that Windows does not provide: $($notSystem -join ', ')") }
    switch ($v) {
      'avx2' { if ($res.ExitCode -ne 0) { $failures.Add("avx2 exited $($res.ExitCode), expected 0") } }
      'avx512' {
        if ($res.ExitCode -eq 0) { $fp['avx512_usable'] = $true }
        if ($fp.avx512f -and $res.ExitCode -eq 3) {
          # Windows reports AVX-512F only; the engine also needs CD, VL, DQ and BW. A clean refusal is the engine doing its job.
          if (-not ($err -and [string]$err.message)) { $failures.Add('avx512 exited 3 but printed no clear message') }
          else { $lines += '   note: this CPU has AVX-512F but the avx512 engine refused it (it needs more AVX-512 features); the avx512 rows are left out'; $fp['avx512_usable'] = $false }
        }
        elseif ($fp.avx512f) { if ($res.ExitCode -ne 0) { $failures.Add("avx512 exited $($res.ExitCode) on a CPU with AVX-512, expected 0 or 3 with a message") } }
        else {
          if ($res.ExitCode -ne 3) { $failures.Add("avx512 exited $($res.ExitCode) on a CPU without AVX-512, expected 3") }
          elseif (-not ($err -and [string]$err.message)) { $failures.Add('avx512 exited 3 but printed no clear message') }
        }
      }
      'vulkan' {
        if ($res.ExitCode -ne 0) {
          $why = $(if ($notSystem -contains 'vulkan-1.dll') { ' (vulkan-1.dll is not installed: the graphics driver has no Vulkan runtime)' } else { '' })
          $failures.Add("vulkan exited $($res.ExitCode), expected 0$why")
        } else {
          $vk = @($devs | Where-Object { $_.Backend -match 'Vulkan' })
          if ($vk.Count -eq 0) { $failures.Add('vulkan lists no graphics adapter') }
          $dup = @($vk | Group-Object Description | Where-Object { $_.Count -gt 1 })
          if ($dup.Count -gt 0) { $failures.Add("vulkan lists an adapter more than once: $(($dup | ForEach-Object { $_.Name }) -join ', ')") }
          $igpuFp = @($fp.gpus | Where-Object { $_.integrated })
          if ($igpuFp.Count -gt 0 -and @($vk | Where-Object { $_.Kind -match 'integrated|igpu|iGPU' }).Count -eq 0) { $lines += '   note: Windows reports an integrated graphics adapter but the engine lists none as integrated; see the device lines above' }
        }
      }
    }
    $lines += ''
  }
  if ($fp.Contains('avx512_usable')) { Write-PackJson -Path (Join-Path ([string]$ctx.work_dir) 'fingerprint.json') -Value $fp }
  # The AVX-512 build counts only where the engine ran it. When it refused, the row for it is NOT APPLICABLE on this CPU
  # (never PASS): record which AVX-512 features the CPU has and what the engine said.
  $avx512Have = @($cpuFeatures | Where-Object { $_ -match '^avx512' })
  $avx512Applicable = ($measured.Contains('avx512') -and [int]$measured['avx512'].exit -eq 0)
  if (-not $avx512Applicable) {
    $lines += ('avx512 build: NOT APPLICABLE on this CPU (not a pass). AVX-512 features the CPU has: {0}. The engine said: {1}' -f $(if ($avx512Have.Count -gt 0) { $avx512Have -join ', ' } else { 'none' }), $(if ($avx512Message) { $avx512Message } else { 'no message' }))
  }
  $lines += 'summary: ' + (($measured.Keys | ForEach-Object { "$_ exit $($measured[$_].exit)" }) -join '; ')
  $evidence = Join-Path ([string]$ctx.gate_dir) 'engines-info.txt'
  Write-WbEvidence -Path $evidence -Lines $lines -MachineLabel ([string]$ctx.machine)
  $status = $(if ($failures.Count -eq 0) { 'PASS' } else { 'FAIL' })
  $head = $(if ($failures.Count -eq 0) { ($measured.Keys | ForEach-Object { "$_ exit $($measured[$_].exit)" }) -join '; ' } else { $failures -join '; ' })
  New-PackResult -Id 'engines-info' -Status $status -Headline $head -Measured ([ordered]@{ variants = $measured; cpu = $fp.cpu; avx512f = $fp.avx512f; avx512_applicable = $avx512Applicable; avx512_features_present = $avx512Have; avx512_engine_message = $avx512Message; gpus = $fp.gpus }) -Threshold 'avx2 exit 0; avx512 exit 0 only with AVX-512 else 3 with a message; vulkan exit 0 and each adapter once' -Evidence @($evidence) -Quick ([bool]$ctx.quick)
}
