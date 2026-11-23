# Tests for scripts/lib/GateData.psm1: no gate, script or pack entry point may run on the real app data folder.
# Run: powershell -NoProfile -ExecutionPolicy Bypass -File scripts/tests/GateData.Tests.ps1
# The "real" folder is a temporary one here (APPDATA and LOCALAPPDATA are redirected for this process only), so the
# test never touches the operator's own files.
# ASCII only; Windows PowerShell 5.1.
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$scripts = Split-Path -Parent $here
Import-Module (Join-Path $scripts 'lib\GateData.psm1') -Force
Import-Module (Join-Path $scripts 'lib\GateRun.psm1') -Force
$fails = 0
function Check([string]$Name, [bool]$Ok) {
  if ($Ok) { Write-Host "ok   $Name" } else { Write-Host "FAIL $Name"; $script:fails++ }
}
function Throws([scriptblock]$Body) { try { & $Body | Out-Null; $false } catch { $true } }
function Message([scriptblock]$Body) { try { & $Body | Out-Null; '' } catch { $_.Exception.Message } }

$tmp = Join-Path ([IO.Path]::GetTempPath()) ('wb-gatedata-test-' + [guid]::NewGuid().ToString('N'))
$fakeRoaming = Join-Path $tmp 'Roaming'
$fakeLocal = Join-Path $tmp 'Local'
New-Item -ItemType Directory -Force -Path $fakeRoaming, $fakeLocal | Out-Null
$saveAppData = $env:APPDATA; $saveLocal = $env:LOCALAPPDATA; $saveGate = $env:WB_GATE_DATA_DIR
$env:APPDATA = $fakeRoaming; $env:LOCALAPPDATA = $fakeLocal
try {
  $real = Join-Path $fakeRoaming 'in.edu.bmu.stelmar.voiceworkbench'
  New-Item -ItemType Directory -Force -Path $real | Out-Null
  Set-Content -LiteralPath (Join-Path $real 'settings.json') -Value '{"recordings_root":"D:\\STELMAR-Transcripts"}' -Encoding ASCII
  Set-Content -LiteralPath (Join-Path $real 'capability.json') -Value '{"tier":"G"}' -Encoding ASCII
  Set-Content -LiteralPath (Join-Path $real 'verified-models.json') -Value '{"models":[]}' -Encoding ASCII

  # ---- The runner refuses the real folder -------------------------------------------------------------------
  Check 'the real folder is recognised' (Test-WbRealAppDataDir $real)
  Check 'a folder inside the real folder is recognised' (Test-WbRealAppDataDir (Join-Path $real 'check\x'))
  Check 'the real folder is recognised with a trailing slash and other case' (Test-WbRealAppDataDir ($real.ToUpperInvariant() + '\'))
  Check 'the real Local folder (logs, WebView2) is recognised' (Test-WbRealAppDataDir (Join-Path $fakeLocal 'in.edu.bmu.stelmar.voiceworkbench\EBWebView'))
  Check 'a folder of a gate run is not the real folder' (-not (Test-WbRealAppDataDir (Join-Path $tmp 'work\run1\appdata')))
  Check 'a sibling that only starts with the same letters is not the real folder' (-not (Test-WbRealAppDataDir ($real + '-gate')))

  $link = Join-Path $tmp 'alias'
  cmd /c mklink /J "$link" "$real" | Out-Null
  Check 'a junction into the real folder is recognised' (Test-WbRealAppDataDir (Join-Path $link 'sub'))
  cmd /c rmdir "$link" | Out-Null

  $env:WB_GATE_DATA_DIR = $real
  Check 'the runner check throws when WB_GATE_DATA_DIR is the real folder' (Throws { Assert-WbGateDataEnvNotReal -Gate 'x' })
  Check 'the refusal says what it is and that nothing was started' ((Message { Assert-WbGateDataEnvNotReal -Gate 'x' }) -match 'GATE REFUSED.*Nothing was started')
  $ran = $false
  Check 'Invoke-WbGate refuses before the gate body runs' (((Message { Invoke-WbGate -Name 'refuse-test' -Body { $script:ran = $true } }) -match 'GATE REFUSED') -and -not $ran)
  Check 'the gate build guard throws when WB_GATE_DATA_DIR is the real folder' (Throws { Assert-WbGateDataDir -Gate 'x' })
  # The order the gate scripts use: GateRun first, then GateData with -Force. Both must keep working.
  Import-Module (Join-Path $scripts 'lib\GateRun.psm1') -Force
  Import-Module (Join-Path $scripts 'lib\GateData.psm1') -Force
  Check 'after GateRun then GateData -Force, Invoke-WbGate still refuses and the helpers are still there' (((Message { Invoke-WbGate -Name 'refuse-test' -Body { $script:ran = $true } }) -match 'GATE REFUSED') -and (Get-Command Set-WbGateDataDir -ErrorAction SilentlyContinue) -and (Get-Command Assert-WbGateDataDir -ErrorAction SilentlyContinue))
  Check 'Set-WbGateDataDir refuses the real folder' (Throws { Set-WbGateDataDir -Dir $real })
  Check 'the real settings.json was not touched by the refusals' ((Get-Content -LiteralPath (Join-Path $real 'settings.json') -Raw) -match 'STELMAR-Transcripts')
  Remove-Item Env:\WB_GATE_DATA_DIR
  Check 'the runner check passes when WB_GATE_DATA_DIR is not set' (-not (Throws { Assert-WbGateDataEnvNotReal -Gate 'x' }))
  Check 'the gate build guard throws when WB_GATE_DATA_DIR is not set' (Throws { Assert-WbGateDataDir -Gate 'x' })
  $own = Join-Path $tmp 'work\run1\appdata'
  Check 'Set-WbGateDataDir accepts a folder of the run and sets the variable' ((Set-WbGateDataDir -Dir $own) -and (Test-Path -LiteralPath $own) -and $env:WB_GATE_DATA_DIR)
  Check 'the gate build guard passes once the folder is set' (-not (Throws { Assert-WbGateDataDir -Gate 'x' }))
  Remove-Item Env:\WB_GATE_DATA_DIR

  # The pack runner itself stops before any gate starts.
  $runner = Get-Content -LiteralPath (Join-Path $scripts 'gate-pack\run-gates.ps1') -Raw
  $iGuard = $runner.IndexOf('Assert-WbGateDataEnvNotReal')
  $iStart = $runner.IndexOf('Invoke-PackUnblock')
  Check 'run-gates.ps1 checks the app data folder before it unblocks files or starts a gate' ($iGuard -gt 0 -and $iStart -gt $iGuard)

  # ---- Release-app scripts: save, compare and restore ---------------------------------------------------------
  $before = Get-WbAppDataHashes -Dir $real
  $saved = Save-WbRealAppData -SaveDir (Join-Path $tmp 'saved') -Dir $real
  Set-Content -LiteralPath (Join-Path $real 'settings.json') -Value '{"recordings_root":"D:\\STELMAR-Gate\\dryrun"}' -Encoding ASCII
  Remove-Item -LiteralPath (Join-Path $real 'verified-models.json')
  $lines = @(Restore-WbRealAppData -Saved $saved)
  $after = Get-WbAppDataHashes -Dir $real
  Check 'a changed settings.json is put back byte for byte' ($before['settings.json'] -eq $after['settings.json'])
  Check 'a deleted verified-models.json is put back' ($before['verified-models.json'] -eq $after['verified-models.json'])
  Check 'an untouched capability.json is reported unchanged' (($before['capability.json'] -eq $after['capability.json']) -and (($lines -join "`n") -match 'capability.json : unchanged'))
  Check 'what the run changed is named in the report' (($lines -join "`n") -match 'settings.json : the run had changed it')

  Remove-Item -LiteralPath (Join-Path $real 'capability.json'), (Join-Path $real 'verified-models.json') -Force
  $saved2 = Save-WbRealAppData -SaveDir (Join-Path $tmp 'saved2') -Dir $real
  Set-Content -LiteralPath (Join-Path $real 'capability.json') -Value '{}' -Encoding ASCII
  $lines2 = @(Restore-WbRealAppData -Saved $saved2)
  Check 'a file the run created is removed again' (-not (Test-Path -LiteralPath (Join-Path $real 'capability.json')) -and (($lines2 -join "`n") -match 'capability.json : the run had created it'))

  # ---- Every entry point that starts the gate build sets WB_GATE_DATA_DIR ---------------------------------------
  # Each of these scripts starts the gate build of the app (or the emulator around it). A new script that sets a
  # WB_GATE_ hook variable or starts the gate build must be listed here, and must carry the guard.
  $entry = [ordered]@{
    'gates\recorder-gate.ps1'            = 'Set-WbGateDataDir'
    'gates\recorder-gate-inner.ps1'      = 'Assert-WbGateDataDir'
    'gates\kill-gate.ps1'                = 'Set-WbGateDataDir'
    'gates\rename-engines.ps1'           = 'Set-WbGateDataDir'
    'gates\day-sim.ps1'                  = 'Set-WbGateDataDir'
    'gates\system-check-gate.ps1'        = 'Set-WbGateDataDir'
    'gate-pack\gates\quick-check.ps1'    = 'Set-WbGateDataDir'
    'gate-pack\gates\full-check.ps1'     = 'Set-WbGateDataDir'
    'gate-pack\gates\mic-run.ps1'        = 'Set-WbGateDataDir'
    'gate-pack\gates\supervisor-kill.ps1' = 'Set-WbGateDataDir'
    'offline-check.ps1'                  = 'Save-WbRealAppData'
    'offline-session-check.ps1'          = 'Save-WbRealAppData'
    'repro-webview2-relaunch.ps1'        = 'Save-WbRealAppData'
  }
  foreach ($k in $entry.Keys) {
    $text = Get-Content -LiteralPath (Join-Path $scripts $k) -Raw
    Check "$k uses $($entry[$k])" ($text.Contains($entry[$k]))
  }
  # The other direction: a script that sets a gate hook or starts the gate build and is not in the list is a hole.
  $known = @($entry.Keys | ForEach-Object { $_.ToLowerInvariant() })
  $startsApp = 'WB_GATE_START\b|WB_GATE_CHECK\b|WB_GATE_DAY_PLAN\b|WB_GATE_RECOVERY\b|Start-WbApp\b|Start-Process\s+-FilePath\s+\$(Exe|paths\.GateExe)\b'
  $holes = @()
  foreach ($f in @(Get-ChildItem -Path $scripts -Recurse -Include *.ps1 -File | Where-Object { $_.FullName -notmatch '\\tests\\' })) {
    $rel = $f.FullName.Substring($scripts.Length + 1).ToLowerInvariant()
    if ($known -contains $rel) { continue }
    $t = Get-Content -LiteralPath $f.FullName -Raw
    if ($t -match $startsApp -and $t -notmatch 'Set-WbGateDataDir|Assert-WbGateDataDir|Save-WbRealAppData') { $holes += $rel }
  }
  Check ('no other script starts the gate build without the guard' + $(if ($holes.Count) { ' (' + ($holes -join ', ') + ')' } else { '' })) ($holes.Count -eq 0)
}
finally {
  $env:APPDATA = $saveAppData; $env:LOCALAPPDATA = $saveLocal
  if ($saveGate) { $env:WB_GATE_DATA_DIR = $saveGate } else { Remove-Item Env:\WB_GATE_DATA_DIR -ErrorAction SilentlyContinue }
  Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue
}
if ($fails) { Write-Host "$fails check(s) failed"; exit 1 }
Write-Host 'all passed'
