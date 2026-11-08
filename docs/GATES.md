# Gate ledger

One row per Verification item that names a machine. Statuses and rules are in `docs/LEGION-FIRST.md` section 3. Claude Code adds rows at the end of each phase; `scripts/ingest-gate-results.ps1` updates them from laptop runs. Evidence paths point to logs committed under `docs/evidence/` (or to the pull request).

## Machines

| Name | Make and model | CPU | RAM | GPUs and driver versions | First used |
| --- | --- | --- | --- | --- | --- |
| Legion | Lenovo Legion Pro 5 16IRX9 | Core i9-14900HX (8P + 16E, 32 threads) | 32 GB | RTX 4070 Laptop 8 GB; Intel UHD | 8 Nov 2026 |
| Low-baseline laptop | (fill in on arrival) | | 16 GB | | |
| Mid laptop | (fill in on arrival) | | 16 GB | | |

## Gates

| Phase | Gate item | Machine or profile | Status | Evidence | Date |
| --- | --- | --- | --- | --- | --- |
| 0 | Shell opens with no network | Legion | PASS | `docs/evidence/phase-0/offline-check-offline.txt` (Wi-Fi off, Ethernet unplugged: 3 of 3 runs rendered, median 133 ms to window and 734 ms to render against 126 ms and 683 ms online, no non-loopback TCP from the app or its WebView2 processes; Record screen with Hindi, Urdu and mixed text in `record-offline.png`); baseline `offline-check-online.txt`. RTSS and MSI Afterburner were closed for the run (see the Phase 1 open item) | 8 Nov 2026 |

## Open items

Problems found while checking a gate that belong to a later phase. Each stays open until that phase closes it.

| Found in | Belongs to | Item | Status | Evidence | Date |
| --- | --- | --- | --- | --- | --- |
| 0 | 8 | WebView2 runtime outbound TLS with network on: the msedgewebview2.exe browser process started by the app opens TCP 443 connections to Microsoft IP addresses (40.99.x, 40.100.x, 40.104.x, 52.98.x seen so far) about 4 s after every launch, with no user action (app code, crates and CSP are clean). The connections come from the browser process itself, not Chromium's network-service utility process, so a switch that only affects the network service may not stop them; Phase 8 must confirm any fix with the watched run. Hostnames not recovered: the Legion resolves names over DNS-over-HTTPS, so the DNS client cache stays empty; Phase 8 can read them from the TLS SNI with `pktmon` (built in, needs admin). Closed when the Phase 8 offline proof shows zero non-loopback TCP or UDP from the app and its WebView2 processes on a connected network (`docs/prompts/phase-8-hardening-and-release.md` lines 10, 18 and 26). Until then, offline audits on a connected Legion treat these connections as this known item | OPEN | `docs/evidence/phase-0/webview2-hosts.txt` | 8 Nov 2026 |
| 0 | 1 | Recover from WebView2 process failure: handle ProcessFailed (browser, renderer or GPU exit), reload or recreate the webview, log it, show a message after repeated failures; recording unaffected. Seen on the Legion: the WebView2 browser process crashed a few seconds after a normal render in about 1 of 20 to 40 launches and left a white window while the app kept running; all seven crash dumps fault in RTSSHooks64.dll (RivaTuner Statistics Server's window hook), with no frame in our code. Phase 1 kill tests get a reproduction case: kill the WebView2 browser process while recording and check the webview recovers and the recording continues | OPEN | `docs/evidence/phase-0/webview2-dump-analysis.txt`, `webview2-crash-reports.txt`, `webview2-relaunch-repro.txt`, `webview2-repro/` | 8 Nov 2026 |
| 0 | 3 | The system check detects known window-hook injectors (RTSS and RTSSHooks64.dll, MSI Afterburner, other overlay tools) and shows a warning on the System screen | OPEN | `docs/evidence/phase-0/webview2-dump-analysis.txt` | 8 Nov 2026 |
| 0 | 8 | The setup-week checklist and runbook say room machines must not run overlay or monitoring tools (RTSS, MSI Afterburner and similar) during the event | OPEN | `docs/evidence/phase-0/webview2-dump-analysis.txt` | 8 Nov 2026 |
