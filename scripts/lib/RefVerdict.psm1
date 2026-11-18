<#
  RefVerdict.psm1: what the recorder gates conclude when the app's recording is compared with TWO
  independent captures of the same input: the cpal reference (the same audio library as the app) and
  a raw WASAPI reference (no cpal). Also the detector counts (zero_gap, repeated_block) read from
  session.log and matched against what the recorder's detectors find in both references.

  The table (Equal = aligned and no differing frames):

    app = cpal, app = WASAPI            PASS
    app = WASAPI, app != cpal           INVALID  the cpal reference is at fault; the app matches the independent capture. Repeat the run.
    app = cpal, app != WASAPI           FAIL     the app and cpal agree with each other and not with Windows' own capture: a fault at the cpal level, which the recorder shares. A recorder FAIL.
    app != cpal, app != WASAPI,
      cpal = WASAPI                     FAIL     two agreeing captures differ from the app: the app lost or changed audio.
    app != cpal, app != WASAPI,
      cpal != WASAPI                    INVALID  the references disagree: no verdict.
    a reference missing or not aligned  INVALID  no verdict without both references.

  Windows PowerShell 5.1, ASCII only.
#>

function Test-PackCompareEqual {
  param($Compare)
  if ($null -eq $Compare) { return $null }
  $aligned = $Compare.PSObject.Properties['aligned']
  $equal = $Compare.PSObject.Properties['equal']
  if ($null -eq $aligned -or $null -eq $equal) { return $null }
  if (-not [bool]$aligned.Value) { return $null }
  [bool]$equal.Value
}

# Pure. Each argument is the compare.json of one pair, or $null when the pair could not be compared.
function Get-PackRefVerdict {
  [CmdletBinding()]
  param($AppCpal, $AppWasapi, $CpalWasapi)
  $ac = Test-PackCompareEqual $AppCpal
  $aw = Test-PackCompareEqual $AppWasapi
  $cw = Test-PackCompareEqual $CpalWasapi
  $name = { param($v) if ($null -eq $v) { 'not comparable' } elseif ($v) { 'equal' } else { 'DIFFERENT' } }
  $table = @(
    ('  app against the cpal reference: {0}' -f (& $name $ac)),
    ('  app against the WASAPI reference: {0}' -f (& $name $aw)),
    ('  cpal reference against the WASAPI reference: {0}' -f (& $name $cw))
  )
  if ($null -eq $ac) { return [pscustomobject]@{ Verdict = 'INVALID'; Class = 'cpal-reference-missing'; Reason = 'the app could not be compared with the cpal reference (missing or not aligned)'; Lines = $table } }
  if ($null -eq $aw) { return [pscustomobject]@{ Verdict = 'INVALID'; Class = 'wasapi-reference-missing'; Reason = 'the app could not be compared with the WASAPI reference (missing or not aligned): no verdict without both references'; Lines = $table } }
  if ($ac -and $aw) { return [pscustomobject]@{ Verdict = 'PASS'; Class = 'both-equal'; Reason = 'the app equals both independent references sample for sample'; Lines = $table } }
  if ($aw -and -not $ac) { return [pscustomobject]@{ Verdict = 'INVALID'; Class = 'cpal-reference-fault'; Reason = 'the app equals the WASAPI reference but not the cpal reference: the cpal reference is at fault, not the recorder; repeat the run'; Lines = $table } }
  if ($ac -and -not $aw) { return [pscustomobject]@{ Verdict = 'FAIL'; Class = 'shared-cpal-fault'; Reason = 'the app equals the cpal reference but not the WASAPI reference: the app and cpal agree with each other and not with Windows'' own capture, a fault at the cpal level that the recorder shares'; Lines = $table } }
  if ($cw -eq $true) { return [pscustomobject]@{ Verdict = 'FAIL'; Class = 'app-differs'; Reason = 'the cpal and WASAPI references agree with each other and both differ from the app: the app lost or changed audio'; Lines = $table } }
  [pscustomobject]@{ Verdict = 'INVALID'; Class = 'references-disagree'; Reason = 'the app differs from both references and the references differ from each other: no verdict'; Lines = $table }
}

# ---- Detector counts from session.log ---------------------------------------------------------------

# Pure. Lines of session.log to the firings of the two detectors: Kind, AudioMs, LenMs.
function ConvertFrom-PackSessionLog {
  [CmdletBinding()]
  param([string[]]$Lines)
  $out = New-Object System.Collections.Generic.List[object]
  foreach ($l in @($Lines)) {
    if ($l -match '^\S+\s+(zero_gap|repeated_block)\s+audio_ms=(\d+)(.*)$') {
      $kind = $Matches[1]; $ms = [double]$Matches[2]; $rest = [string]$Matches[3]
      $len = 0.0
      if ($rest -match 'len_ms=([0-9.]+)') { $len = [double]::Parse($Matches[1], [System.Globalization.CultureInfo]::InvariantCulture) }
      elseif ($rest -match 'len_frames=(\d+).*rate=(\d+)') { $len = 1000.0 * [double]$Matches[1] / [double]$Matches[2] }
      $out.Add([pscustomobject]@{ Kind = $kind; AudioMs = $ms; LenMs = $len })
    }
  }
  @($out.ToArray())
}

# Pure. Matches the app's firings of one kind against the findings of the two references, mapped to
# the app's audio clock (-OffsetMs: app time = reference time + offset).
#   Firings       how many times the detector fired
#   Matched       fired where at least one reference also shows the break (within ToleranceMs)
#   MatchedBoth   fired where both references show it
#   FalseAlarms   fired where neither reference shows anything
#   RealMissed    a reference finding (in both) that the detector did not fire for
#   PerHour       firings per hour of audio
function Get-PackDetectorRates {
  [CmdletBinding()]
  param(
    $Firings, $CpalFindings, $WasapiFindings,
    [double]$OffsetCpalMs = 0, [double]$OffsetWasapiMs = 0,
    [double]$AudioMs = 0, [double]$ToleranceMs = 250
  )
  $f = @($Firings | ForEach-Object { [double]$_.AudioMs })
  $c = @($CpalFindings | ForEach-Object { [double]$_.AudioMs + $OffsetCpalMs })
  $w = @($WasapiFindings | ForEach-Object { [double]$_.AudioMs + $OffsetWasapiMs })
  $near = { param($t, $set) @($set | Where-Object { [math]::Abs($_ - $t) -le $ToleranceMs }).Count -gt 0 }
  $matched = 0; $both = 0; $alarms = 0
  foreach ($t in $f) {
    $inC = & $near $t $c; $inW = & $near $t $w
    if ($inC -or $inW) { $matched++ } else { $alarms++ }
    if ($inC -and $inW) { $both++ }
  }
  $missed = 0
  foreach ($t in $c) {
    if ((& $near $t $w) -and -not (& $near $t $f)) { $missed++ }
  }
  [pscustomobject]@{
    Firings = $f.Count; Matched = $matched; MatchedBoth = $both; FalseAlarms = $alarms; RealMissed = $missed
    ReferenceFindings = [math]::Max($c.Count, $w.Count)
    PerHour = $(if ($AudioMs -gt 0) { $f.Count / ($AudioMs / 3600000.0) } else { $null })
  }
}

# One verdict line per detector, for the summary and the morning report.
function Get-PackDetectorLine {
  [CmdletBinding()]
  param([string]$Kind, $Rates, [string]$Conditions = '')
  $per = $(if ($null -ne $Rates.PerHour) { ('{0:n2} per hour' -f $Rates.PerHour) } else { 'rate unknown' })
  ('{0}: fired {1} time(s) ({2}); matched by a reference {3}, by both {4}; false alarms {5}; real dropouts the detector missed {6}{7}' -f $Kind, $Rates.Firings, $per, $Rates.Matched, $Rates.MatchedBoth, $Rates.FalseAlarms, $Rates.RealMissed, $(if ($Conditions) { "; $Conditions" } else { '' }))
}

Export-ModuleMember -Function Test-PackCompareEqual, Get-PackRefVerdict, ConvertFrom-PackSessionLog, Get-PackDetectorRates, Get-PackDetectorLine
