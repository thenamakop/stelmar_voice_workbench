# Documentation index

| Document | Read it for |
| --- | --- |
| [`PLAN.md`](PLAN.md) | The full plan: decisions D1 to D14, machine tiers, system check, modes and the Start Session dialog, languages and code-switching, models, accuracy, architecture, recording, screens, offline guarantee, build, phases and gates, runbook, risks, sources |
| [`LEGION-FIRST.md`](LEGION-FIRST.md) | Work from 8 to 17 November 2026, before the test laptops arrive: day-by-day plan, Legion emulation profiles, gate statuses |
| [`GATES.md`](GATES.md) | The gate ledger: every machine-named Verification item and its status |
| [`SETUP.md`](SETUP.md) | Build tools for the Legion and what `scripts/setup-legion.ps1` checks |
| [`TEST-AUDIO.md`](TEST-AUDIO.md) | Why and how to record and transcribe the local test sets (scheduled after Phase 4) |
| [`programme.json`](programme.json) | The STELMAR 2027 programme that seeds the Sessions screen |
| [`../AGENTS.md`](../AGENTS.md) | Rules every agent and contributor follows |
| [`../CONTRIBUTING.md`](../CONTRIBUTING.md) | Branches, commits, reviews, no AI attribution |
| [`../design/README.md`](../design/README.md) | Design tokens, fonts, assets and mockups |
| [`../models/README.md`](../models/README.md) | Candidate models, conversion and quantisation, model packs |

## Phase prompts

Each prompt has Context, Goal, Steps, Verification and If Blocked sections. Run them with `/run-phase <n>`.

| Prompt | Phase |
| --- | --- |
| [`prompts/phase-0-shell-and-design.md`](prompts/phase-0-shell-and-design.md) | 0: App shell in the STELMAR design |
| [`prompts/phase-1-audio-and-backup.md`](prompts/phase-1-audio-and-backup.md) | 1: Recording, backup, Legion emulation |
| [`prompts/phase-2-engines-and-models.md`](prompts/phase-2-engines-and-models.md) | 2: Engines, model registry, supervisor |
| [`prompts/phase-3-system-check.md`](prompts/phase-3-system-check.md) | 3: System check, Start Session dialog |
| [`prompts/phase-4-post-processing-queue.md`](prompts/phase-4-post-processing-queue.md) | 4: Queue, hybrid scheduler, night pass |
| [`prompts/phase-5a-eval-groundwork.md`](prompts/phase-5a-eval-groundwork.md) | 5a: Evaluation groundwork (parallel track) |
| [`prompts/phase-5-languages-and-eval.md`](prompts/phase-5-languages-and-eval.md) | 5: Languages, code-switching, model choice |
| [`prompts/phase-6-review-and-export.md`](prompts/phase-6-review-and-export.md) | 6: Sessions, Review, exports |
| [`prompts/phase-7-live-and-captions.md`](prompts/phase-7-live-and-captions.md) | 7: Live text and captions |
| [`prompts/phase-8-hardening-and-release.md`](prompts/phase-8-hardening-and-release.md) | 8: Hardening, offline proof, release |
| [`prompts/laptop-gate-pack.md`](prompts/laptop-gate-pack.md) | Gate pack for the laptops (`/run-phase gate-pack`) |
