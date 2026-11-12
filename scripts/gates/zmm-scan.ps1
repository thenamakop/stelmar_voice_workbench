<#
.SYNOPSIS
  Phase 2 gate 11: the AVX-512 scan of the portable engines, evidence written by this script.

.DESCRIPTION
  Runs scripts\check-no-zmm.ps1 on the avx2 engine (expected: no zmm register), on the vulkan engine
  (compiled for the CPU with the avx2 flags, so it must have none either) and, as the positive control, on
  the avx512 engine (expected: a zmm register, which proves the scan can see them). Writes
  docs\evidence\phase-2\no-zmm.txt. Evidence files are written by scripts only, never edited by hand.
#>
[CmdletBinding()]
param(
  [string]$OutDir,
  [string]$Machine = 'Legion'
)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo = (Resolve-Path (Join-Path $here '..\..')).Path
if (-not $OutDir) { $OutDir = Join-Path $repo 'docs\evidence\phase-2' }
Import-Module (Join-Path $repo 'scripts\lib\PsEnv.psm1') -Force
Set-WbCleanPSModulePath
Set-WbInvariantCulture
Import-Module (Join-Path $repo 'scripts\lib\GateRun.psm1') -Force
Import-Module (Join-Path $repo 'scripts\lib\Redact.psm1') -Force
$inv = [Globalization.CultureInfo]::InvariantCulture

Assert-WbNoOverlayTools
Assert-WbNoBuilds

$triple = 'x86_64-pc-windows-msvc'
$scan = Join-Path $repo 'scripts\check-no-zmm.ps1'
function Invoke-Scan([string]$Variant, [string]$Expect) {
  $exe = Join-Path $repo "src-tauri\binaries\wb-engine-$Variant-$triple.exe"
  $r = Invoke-WbNative -FilePath 'powershell.exe' -ArgumentList @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $scan, '-Exe', $exe, '-Expect', $Expect)
  $text = ($r.Output -join "`n")
  $m = [regex]::Match($text, '^(?<name>\S+) \((?<sha>[0-9a-f]{64})\): (?<what>.*)$', 'Multiline')
  if (-not $m.Success) { throw "unexpected output of check-no-zmm for ${Variant}: $text $($r.Error -join ' ')" }
  $what = $m.Groups['what'].Value
  $lines = [regex]::Match($what, '(\d+) lines').Groups[1].Value
  $n = ([int64]$lines).ToString('N0', $inv)
  $ok = ($r.ExitCode -eq 0)
  if ($what -match 'a zmm register appears after \d+ lines of disassembly: (?<at>[0-9A-F]+): (?<ins>\S+)\s+(?<ops>zmm\d+),zmmword ptr') {
    $detail = "a zmm register appears after $n lines of disassembly ($($Matches['at']): $($Matches['ins']) $($Matches['ops']),zmmword ptr [...])"
  } else {
    $detail = "no zmm register in $n lines of disassembly"
  }
  [pscustomobject]@{ Variant = $Variant; Name = $m.Groups['name'].Value; Sha = $m.Groups['sha'].Value; Detail = $detail; Ok = $ok; Expect = $Expect }
}

$a = Invoke-Scan 'avx2' 'Absent'
$v = Invoke-Scan 'vulkan' 'Absent'
$c = Invoke-Scan 'avx512' 'Present'
$pass = $a.Ok -and $v.Ok -and $c.Ok
$lines = @(
  'AVX-512 instruction scan of the portable engines (Phase 2, gate 11)',
  "date: $((Get-Date).ToString('yyyy-MM-dd HH:mm', $inv)) (IST on the Legion); machine: $Machine; tool: scripts/check-no-zmm.ps1 run by scripts/gates/zmm-scan.ps1 (dumpbin /disasm, streamed, stops at the first zmm register); overlay tools closed: yes (no timing involved)",
  'CI runs the same script on the avx2 build (.github/workflows/ci.yml, "no zmm" step). The vulkan engine is also compiled for the CPU with the avx2 flags (GGML_NATIVE=OFF), so it is scanned here too; CI builds avx2 only.',
  '',
  "1. $($a.Name) (sha256 $($a.Sha)), expected: no zmm register",
  "   result: $($a.Detail). $(if ($a.Ok) { 'OK' } else { 'FAIL' })",
  "2. $($v.Name) (sha256 $($v.Sha)), expected: no zmm register",
  "   result: $($v.Detail). $(if ($v.Ok) { 'OK' } else { 'FAIL' })",
  "3. positive control, $($c.Name) (sha256 $($c.Sha)), expected: a zmm register",
  "   result: $($c.Detail). $(if ($c.Ok) { 'OK; the scan can see them.' } else { 'FAIL' })",
  "verdict: $(if ($pass) { 'PASS' } else { 'FAIL' })"
)
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$path = Join-Path $OutDir 'no-zmm.txt'
Write-WbEvidence -Path $path -Lines $lines
Write-Host "evidence: $path"
$lines | Select-Object -Last 8
if (-not $pass) { exit 1 }
