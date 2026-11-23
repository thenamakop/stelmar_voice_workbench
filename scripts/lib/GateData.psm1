# The app data folder of a gate run. A gate that starts the gate build of the app must give it a folder of its own
# (WB_GATE_DATA_DIR, inside the gate's work folder); otherwise the app reads and writes the operator's REAL
# settings.json, capability.json and verified-models.json (%APPDATA%\in.edu.bmu.stelmar.voiceworkbench). On
# 23 Nov 2026 a recorder gate left the operator's recordings folder pointing into D:\STELMAR-Gate.
#
# The release build has no WB_GATE_DATA_DIR (the hooks are not compiled into it), so a script that must run the
# release app cannot move its data; Save-WbRealAppData / Restore-WbRealAppData put the three files back byte for
# byte afterwards and say what changed.
#
# ASCII only; Windows PowerShell 5.1.

$script:Identifier = 'in.edu.bmu.stelmar.voiceworkbench'
$script:ProtectedFiles = @('settings.json', 'capability.json', 'verified-models.json')

# The operator's real folders: Roaming (settings, capability, model cache) and Local (logs, WebView2).
function Get-WbRealAppDataDirs {
  $dirs = @()
  if ($env:APPDATA) { $dirs += (Join-Path $env:APPDATA $script:Identifier) }
  if ($env:LOCALAPPDATA) { $dirs += (Join-Path $env:LOCALAPPDATA $script:Identifier) }
  $dirs
}

function Get-WbRealAppDataDir {
  Join-Path $env:APPDATA $script:Identifier
}

# The full path with junctions and symbolic links resolved (a folder that exists only through a junction into the
# real folder must not slip past a plain string comparison). Folders that do not exist yet are resolved up to
# their nearest existing parent.
function Resolve-WbRealPath([string]$Path) {
  $full = [IO.Path]::GetFullPath($Path)
  if ($full.StartsWith('\\?\')) { $full = $full.Substring(4) }
  $full = $full.TrimEnd('\')
  $parts = @($full -split '\\')
  if ($parts.Count -lt 2) { return $full }
  $cur = $parts[0] + '\'
  for ($i = 1; $i -lt $parts.Count; $i++) {
    $next = Join-Path $cur $parts[$i]
    $it = Get-Item -LiteralPath $next -Force -ErrorAction SilentlyContinue
    if ($it -and ($it.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
      $target = @($it.Target)[0]
      if ($target) {
        if (-not [IO.Path]::IsPathRooted($target)) { $target = Join-Path $cur $target }
        $next = [IO.Path]::GetFullPath($target).TrimEnd('\')
        if ($next.StartsWith('\\?\')) { $next = $next.Substring(4) }
      }
    }
    $cur = $next
  }
  $cur.TrimEnd('\')
}

# True when $Path is the operator's real app data folder (Roaming or Local), a folder inside it, or reaches one
# through a junction.
function Test-WbRealAppDataDir([string]$Path) {
  if (-not $Path) { return $false }
  $p = Resolve-WbRealPath $Path
  foreach ($real in (Get-WbRealAppDataDirs)) {
    $r = Resolve-WbRealPath $real
    if ($p.Equals($r, [StringComparison]::OrdinalIgnoreCase) -or $p.StartsWith($r + '\', [StringComparison]::OrdinalIgnoreCase)) { return $true }
  }
  $false
}

function Get-WbRealDataRefusal([string]$Gate, [string]$Dir) {
  "GATE REFUSED: $Gate would run on the real app data folder ($Dir resolves to $(Get-WbRealAppDataDir) or a folder inside it). " +
  "A gate run changes settings.json and deletes verified-models.json there. Give the run a folder of its own inside its work folder " +
  "(Set-WbGateDataDir) and start it again. Nothing was started."
}

# Creates the gate's own data folder and points the gate build of the app at it. Refuses a folder that resolves to
# the real one. Returns the folder.
function Set-WbGateDataDir {
  param([Parameter(Mandatory)] [string]$Dir)
  if (Test-WbRealAppDataDir $Dir) { throw (Get-WbRealDataRefusal -Gate 'This gate' -Dir $Dir) }
  New-Item -ItemType Directory -Force -Path $Dir | Out-Null
  $env:WB_GATE_DATA_DIR = (Resolve-WbRealPath $Dir)
  $env:WB_GATE_DATA_DIR
}

# The guard to call right before the gate build of the app is started: WB_GATE_DATA_DIR must be set and must not
# resolve to the real folder.
function Assert-WbGateDataDir {
  param([string]$Gate = 'This gate')
  $d = $env:WB_GATE_DATA_DIR
  if (-not $d -or -not $d.Trim()) {
    throw ("GATE REFUSED: $Gate starts the gate build of the app but WB_GATE_DATA_DIR is not set, so the app would use the real app data folder " +
      "($(Get-WbRealAppDataDir)) and change the operator's settings. Set it with Set-WbGateDataDir to a folder inside the gate's work folder. Nothing was started.")
  }
  if (Test-WbRealAppDataDir $d) { throw (Get-WbRealDataRefusal -Gate $Gate -Dir $d) }
}

# The runner's own check, before any gate starts: a WB_GATE_DATA_DIR that is already set (the shell the person
# started the runner from) must not point at the real folder, because every gate inherits it.
function Assert-WbGateDataEnvNotReal {
  param([string]$Gate = 'The gate runner')
  $d = $env:WB_GATE_DATA_DIR
  if ($d -and $d.Trim() -and (Test-WbRealAppDataDir $d)) { throw (Get-WbRealDataRefusal -Gate $Gate -Dir $d) }
}

function Get-WbFileSha256([string]$Path) {
  $s = [IO.File]::OpenRead($Path)
  try { ([BitConverter]::ToString([Security.Cryptography.SHA256]::Create().ComputeHash($s))).Replace('-', '') } finally { $s.Dispose() }
}

# Hashes of the three protected files in a folder (default: the real one); $null for a file that is not there.
function Get-WbAppDataHashes {
  param([string]$Dir = (Get-WbRealAppDataDir))
  $h = [ordered]@{}
  foreach ($n in $script:ProtectedFiles) {
    $p = Join-Path $Dir $n
    $h[$n] = $(if (Test-Path -LiteralPath $p) { Get-WbFileSha256 $p } else { $null })
  }
  $h
}

# For a script that has to run the RELEASE build of the app (it cannot be redirected): copies the three files to
# $SaveDir first. Refuses while the app is running (its own writes would be lost or kept by mistake).
function Save-WbRealAppData {
  param([Parameter(Mandatory)] [string]$SaveDir, [string]$Dir = (Get-WbRealAppDataDir))
  if (Get-Process -Name 'stelmar-voice-workbench' -ErrorAction SilentlyContinue) {
    throw 'The app is already running. Close it first: this script protects the real app data files while it starts the release app, and a running app would change them.'
  }
  New-Item -ItemType Directory -Force -Path $SaveDir | Out-Null
  $files = [ordered]@{}
  foreach ($n in $script:ProtectedFiles) {
    $p = Join-Path $Dir $n
    if (Test-Path -LiteralPath $p) {
      Copy-Item -LiteralPath $p -Destination (Join-Path $SaveDir $n) -Force
      $files[$n] = [pscustomobject]@{ Existed = $true; Sha256 = (Get-WbFileSha256 $p) }
    } else {
      $files[$n] = [pscustomobject]@{ Existed = $false; Sha256 = $null }
    }
  }
  [pscustomobject]@{ Dir = $Dir; SaveDir = $SaveDir; Files = $files }
}

# Puts the three files back as Save-WbRealAppData found them (a file the run created is removed). Returns lines
# for the evidence: what the run had changed and that it was put back.
function Restore-WbRealAppData {
  param([Parameter(Mandatory)] $Saved)
  $lines = New-Object System.Collections.Generic.List[string]
  foreach ($n in $Saved.Files.Keys) {
    $p = Join-Path $Saved.Dir $n
    $was = $Saved.Files[$n]
    $now = $(if (Test-Path -LiteralPath $p) { Get-WbFileSha256 $p } else { $null })
    if ($was.Existed) {
      if ($now -ne $was.Sha256) {
        Copy-Item -LiteralPath (Join-Path $Saved.SaveDir $n) -Destination $p -Force
        $lines.Add("$n : the run had changed it; restored from the copy taken before the run")
      } else { $lines.Add("$n : unchanged") }
    } else {
      if ($null -ne $now) { Remove-Item -LiteralPath $p -Force; $lines.Add("$n : the run had created it; removed (it was not there before)") }
      else { $lines.Add("$n : not there before, not there after") }
    }
  }
  $lines.ToArray()
}

Export-ModuleMember -Function Get-WbRealAppDataDirs, Get-WbRealAppDataDir, Resolve-WbRealPath, Test-WbRealAppDataDir, Set-WbGateDataDir, Assert-WbGateDataDir, Assert-WbGateDataEnvNotReal, Get-WbFileSha256, Get-WbAppDataHashes, Save-WbRealAppData, Restore-WbRealAppData
