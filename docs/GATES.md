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
| 0 | Shell opens with no network | Legion | | | |
