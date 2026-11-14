# Tests for scripts/lib/PeImports.psm1: the PE import reader, with Windows' own programs as controls.
# Run: powershell -NoProfile -ExecutionPolicy Bypass -File scripts/tests/PeImports.Tests.ps1
# ASCII only; Windows PowerShell 5.1.
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
Import-Module (Join-Path $here '..\lib\PeImports.psm1') -Force
$fails = 0
function Check([string]$Name, [bool]$Ok) {
  if ($Ok) { Write-Host "ok   $Name" } else { Write-Host "FAIL $Name"; $script:fails++ }
}
$sys = Join-Path $env:SystemRoot 'System32'

$k = @(Get-PePeImports -Path (Join-Path $sys 'kernel32.dll'))
Check 'kernel32.dll is read as a PE file with imports' ($k.Count -gt 0)
$n = @(Get-PePeImports -Path (Join-Path $sys 'notepad.exe'))
Check 'notepad.exe imports something and no network library' ($n.Count -gt 0 -and @(Select-PackNetworkDlls $n).Count -eq 0)
$ns = @(Get-PePeImports -Path (Join-Path $sys 'netstat.exe'))
Check 'netstat.exe (the positive control) imports a network library' (@(Select-PackNetworkDlls $ns).Count -gt 0)
Check 'a text file is not a PE file' ($null -eq (Get-PePeImports -Path (Join-Path $here 'PeImports.Tests.ps1')))

$r = Test-PackNetworkImports -Files @((Join-Path $sys 'notepad.exe'), (Join-Path $sys 'netstat.exe'), (Join-Path $here 'PeImports.Tests.ps1'))
Check 'the control is found' $r.ControlOk
Check 'netstat is a finding and notepad is not' (@($r.Findings).Count -eq 1 -and $r.Findings[0].Path -like '*netstat.exe')
Check 'a non-PE file is reported as unreadable, not as clean' (@($r.Unreadable).Count -eq 1)
$bad = Test-PackNetworkImports -Files @((Join-Path $sys 'notepad.exe')) -ControlFiles @((Join-Path $sys 'notepad.exe'))
Check 'with a control that imports nothing network the control fails' (-not $bad.ControlOk)

if ($fails) { Write-Host "$fails failed"; exit 1 }
Write-Host 'PeImports tests passed'
