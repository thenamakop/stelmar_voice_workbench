# Contributing

## Workflow

1. One phase per branch, named `phase-<n>-<slug>` (for example `phase-1-audio-and-backup`), opened as one pull request into `master`.
2. Start each phase with `/run-phase <n>` in Claude Code. Claude proposes a plan; approve it before any file changes.
3. A phase is done when its Verification list passes, the reviewer subagents it names report PASS, and every machine-named gate has a row in `docs/GATES.md`.
4. Before every merge, run:

   ```powershell
   pnpm lint; pnpm typecheck; pnpm test
   cargo clippy --workspace --all-targets -- -D warnings; cargo test --workspace
   ```

5. Never commit model files, datasets, private audio, `gate-pack/` or `.setup/` (all git-ignored).

## Commits

- Write the subject in the imperative, under about 70 characters (`Add crash-safe WAV recorder`), then a blank line and a short body if needed.
- Commit with your own Git identity (`git config user.name` and `user.email`).

## No AI attribution

AI tools help build this project, but the Git history and GitHub never credit them. This applies to Claude Code, Claude, and any other assistant or agent.

| Not allowed | Where it is enforced |
| --- | --- |
| `Co-Authored-By:` trailers naming an AI tool or an AI vendor's address | `.claude/settings.json` (`attribution` set to empty strings), `.githooks/commit-msg`, CI |
| "Generated with ..." lines, robot emoji footers, session links | Same, plus the CI check on pull request descriptions |
| An AI tool as commit author or committer | CI |

Setup: `scripts/setup-legion.ps1` runs `git config core.hooksPath .githooks`. On a fresh clone without the script, run that command once. Never bypass the hook with `--no-verify`.

If a commit with AI attribution reaches a branch anyway, rewrite it before merging (`git commit --amend`, or an interactive rebase for older commits) and force-push the branch. Never rewrite `master`.
