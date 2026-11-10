# Tests for scripts/lib/Detector.psm1: lines up the harness's shared upstream breaks with the
# recorder's repeated_block and zero_gap events. The numbers are the real ones from the 10 Nov 2026
# panic and force-kill runs.
# Run: powershell -NoProfile -ExecutionPolicy Bypass -File scripts/tests/Detector.Tests.ps1
# ASCII only; Windows PowerShell 5.1.
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
Import-Module (Join-Path $here '..\lib\Detector.psm1') -Force
$fails = 0
function Check([string]$Name, [bool]$Ok) {
  if ($Ok) { Write-Host "ok   $Name" } else { Write-Host "FAIL $Name"; $script:fails++ }
}
function Disc([double]$SourceMs, [double]$JumpMs) { [pscustomobject]@{ source_ms = $SourceMs; source_jump_ms = $JumpMs } }
function Ev([string]$Kind, [double]$AudioMs) { [pscustomobject]@{ kind = $Kind; audio_ms = $AudioMs; detail = 'start_frame=1 len_frames=490' } }
function Brk([double]$SourceMs, [double]$UnmatchedMs) {
  [pscustomobject]@{ SourceMs = $SourceMs; UnmatchedMs = $UnmatchedMs; RepeatMs = 10.2; SkippedMs = 0.0 }
}

# Panic run: the signal starts at app audio 2328.02 ms, the break is at source 18.667 ms (81.3 ms
# unmatched), so the span is app 2346.69 to 2428.02 ms; the recorder marked a zero_gap at 2346.58.
$panic = [pscustomobject]@{ sample_rate = 48000; content_start_ms = 2328.0208333333335; content_first_source_frame = 0; discontinuities = @((Disc 100.0 -10.208333333333334)) }
Check 'offset at the start of the signal' ([math]::Abs((Get-WbAppOffsetMs -Analysis $panic -SourceMs 18.667) - 2328.0208) -lt 0.001)
$m = Get-WbDetectorMatches -Breaks @((Brk 18.667 81.333)) -Analysis $panic -Events @((Ev 'zero_gap' 2346.583))
Check 'the panic run break is matched by the zero_gap' ($m.Matched -eq 1 -and $m.Missed -eq 0 -and @($m.FalseAlarms).Count -eq 0)
Check 'the span is mapped to app audio time' ([math]::Abs($m.Rows[0].AppStartMs - 2346.6878) -lt 0.01 -and [math]::Abs($m.Rows[0].AppEndMs - 2428.0208) -lt 0.01)

# Within the tolerance on either side, and outside it.
$m = Get-WbDetectorMatches -Breaks @((Brk 18.667 81.333)) -Analysis $panic -Events @((Ev 'repeated_block' 2480.0))
Check 'an event 52 ms after the span end is outside the +-50 ms tolerance: a miss and a false alarm' ($m.Matched -eq 0 -and $m.Missed -eq 1 -and @($m.FalseAlarms).Count -eq 1)
$m = Get-WbDetectorMatches -Breaks @((Brk 18.667 81.333)) -Analysis $panic -Events @((Ev 'repeated_block' 2470.0))
Check 'an event 42 ms after the span end is inside it' ($m.Matched -eq 1)
$m = Get-WbDetectorMatches -Breaks @((Brk 18.667 81.333)) -Analysis $panic -Events @((Ev 'zero_gap' 2300.0))
Check 'an event 46 ms before the span start is inside it' ($m.Matched -eq 1)

# A miss, a false alarm and other event kinds.
$m = Get-WbDetectorMatches -Breaks @((Brk 18.667 81.333)) -Analysis $panic -Events @()
Check 'no event: a miss' ($m.Matched -eq 0 -and $m.Missed -eq 1)
$m = Get-WbDetectorMatches -Breaks @() -Analysis $panic -Events @((Ev 'zero_gap' 60000.0), (Ev 'opened' 0.0), (Ev 'gap' 100.0))
Check 'an event with no break is a false alarm; other kinds are ignored' ($m.EventCount -eq 1 -and @($m.FalseAlarms).Count -eq 1 -and @($m.Rows).Count -eq 0)
$m = Get-WbDetectorMatches -Breaks @((Brk 18.667 81.333)) -Analysis $panic -Events @((Ev 'zero_gap' 2346.6), (Ev 'zero_gap' 2347.0))
Check 'two events at one break are both matched, none is a false alarm' ($m.Matched -eq 1 -and @($m.Rows[0].Events).Count -eq 2 -and @($m.FalseAlarms).Count -eq 0)

# Force-kill run: three restarts before the break, each moving the source ahead by about 3.5 s.
# Source 398627.417 ms is app audio 383954.7 ms; the recorder marked a zero_gap at 383954.56.
$kill = [pscustomobject]@{
  sample_rate = 48000; content_start_ms = 0.0; content_first_source_frame = 193956
  discontinuities = @((Disc 123906.083 3564.0), (Disc 248370.083 3584.0), (Disc 341606.083 3484.0), (Disc 398690.083 -11.479), (Disc 442306.604 3772.0), (Disc 521981.271 3678.0))
}
$off = Get-WbAppOffsetMs -Analysis $kill -SourceMs 398627.417
Check 'restarts before the break move the offset by their jumps' ([math]::Abs(($off + 14672.75)) -lt 0.01)
$m = Get-WbDetectorMatches -Breaks @((Brk 398627.417 73.333)) -Analysis $kill -Events @((Ev 'zero_gap' 383954.5625))
Check 'the force-kill run break is matched through the restarts' ($m.Matched -eq 1)
# The later restarts (after the break) do not shift it.
Check 'jumps after the break are not applied to it' ([math]::Abs((Get-WbAppOffsetMs -Analysis $kill -SourceMs 100.0) - (0 - 193956.0 * 1000.0 / 48000.0)) -lt 0.001)

# Evidence lines.
$rec = [pscustomobject]@{ position_breaks = 0; timestamp_outliers = 2; position_pair_flags = 4; repeated_blocks = 0; zero_gaps = 1 }
$m = Get-WbDetectorMatches -Breaks @((Brk 18.667 81.333)) -Analysis $panic -Events @((Ev 'zero_gap' 2346.583), (Ev 'repeated_block' 90000.0))
$text = (Get-WbDetectorLines -Check $m -Rec $rec) -join "`n"
Check 'lines state the counts' ($text -match 'found by the harness: 1; matched 1; missed 0; events with no shared break \(false alarms\) 1')
Check 'lines name a false alarm' ($text -match 'FALSE ALARM: repeated_block at 90,000\.0 ms')
Check 'lines give the position counters after the fix' ($text -match 'position breaks 0 .* timestamp outliers 2 .* old pairwise flags 4 .* zero_gap events 1')
$old = (Get-WbDetectorLines -Check $m -Rec ([pscustomobject]@{ position_breaks = 3 })) -join "`n"
Check 'a report from an older build says the counters are not available' ($old -match 'not available')

if ($fails) { Write-Host "$fails failed"; exit 1 }
Write-Host 'Detector tests passed'
