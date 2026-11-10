<#
.SYNOPSIS
  Fails when a portable CPU build contains an AVX-512 instruction (a zmm register).

.DESCRIPTION
  Disassembles the executable with `dumpbin /disasm` and streams the output, stopping at the first
  line that names a zmm register. The `avx2` engine must contain none: a laptop without AVX-512
  would fault on it (PLAN section 11, AGENTS.md non-negotiable 4).

  -Expect Present is the positive control: it succeeds only if a zmm register IS found (run it on the
  `avx512` build to show the scan can see them).
#>
[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)] [string]$Exe,
  [ValidateSet('Absent', 'Present')] [string]$Expect = 'Absent',
  [string]$Dumpbin
)
$ErrorActionPreference = 'Stop'

function Find-Dumpbin {
  $roots = @($env:ProgramFiles, ${env:ProgramFiles(x86)}) | Where-Object { $_ }
  foreach ($r in $roots) {
    $vs = Join-Path $r 'Microsoft Visual Studio'
    if (-not (Test-Path -LiteralPath $vs)) { continue }
    $hit = Get-ChildItem -Path $vs -Recurse -Filter dumpbin.exe -ErrorAction SilentlyContinue |
      Where-Object { $_.FullName -match 'Hostx64\\x64' } | Sort-Object FullName -Descending | Select-Object -First 1
    if ($hit) { return $hit.FullName }
  }
  $null
}

if (-not (Test-Path -LiteralPath $Exe)) { throw "No such file: $Exe" }
if (-not $Dumpbin) { $Dumpbin = Find-Dumpbin }
if (-not $Dumpbin) { throw 'dumpbin.exe not found; install the Visual Studio Build Tools C++ workload.' }

$psi = New-Object System.Diagnostics.ProcessStartInfo
$psi.FileName = $Dumpbin
$psi.Arguments = '/nologo /disasm:nobytes "' + (Resolve-Path -LiteralPath $Exe).Path + '"'
$psi.RedirectStandardOutput = $true
$psi.RedirectStandardError = $true
$psi.UseShellExecute = $false
$psi.CreateNoWindow = $true
$p = [System.Diagnostics.Process]::Start($psi)
$errTask = $p.StandardError.ReadToEndAsync()
$lines = 0L
$found = $null
while ($null -ne ($line = $p.StandardOutput.ReadLine())) {
  $lines++
  if ($line -match '\bzmm\d+\b') { $found = $line.Trim(); break }
}
if ($found) { try { $p.Kill() } catch { } } else { $p.WaitForExit() }
$name = Split-Path -Leaf $Exe
$sha = (Get-FileHash -LiteralPath $Exe -Algorithm SHA256).Hash.ToLowerInvariant()
if ($found) {
  Write-Host "$name ($sha): a zmm register appears after $lines lines of disassembly: $found"
} else {
  Write-Host "$name ($sha): no zmm register in $lines lines of disassembly"
}
if ($Expect -eq 'Absent' -and $found) { Write-Host 'FAIL: this build must not contain AVX-512 instructions.'; exit 1 }
if ($Expect -eq 'Present' -and -not $found) { Write-Host 'FAIL: the positive control found no zmm register; the scan cannot be trusted.'; exit 1 }
Write-Host 'OK'
