---
name: run-phase
description: Run one build phase of the STELMAR Voice Workbench from docs/prompts.
argument-hint: [phase-number]
disable-model-invocation: true
---

Run phase $0 of the STELMAR Voice Workbench build.

1. Read `AGENTS.md` and `docs/PLAN.md` if they are not already in context.
2. Open the file in `docs/prompts/` whose name starts with `phase-$0-` (or `laptop-gate-pack.md` when the argument is `gate-pack`) and read all of it.
3. Check that the previous phase's gate (its Verification list) is met, using `docs/GATES.md`: PASS and PROVISIONAL items count as met, and DEFERRED only for the items `docs/LEGION-FIRST.md` section 3 allows. Phase 5a needs only Phase 0. If the gate isn't met, stop and report what's missing.
4. Propose a short plan for this phase, including which steps need the low-baseline laptop, the mid laptop or the Legion (GPU), and which run under `emu-low` or `emu-mid` until the laptops arrive, and wait for approval before editing files.
5. Create branch `phase-$0-<slug>`, follow the Steps, and run every Verification item. Paste the commands and results, naming the machine or emulation profile for every measurement, and add a row to `docs/GATES.md` for every machine-named item.
6. Ask the reviewer subagents the prompt names to review the change, and fix their findings.
7. Finish with: what changed, verification results, screenshots for UI work, and open issues or decisions needed.
