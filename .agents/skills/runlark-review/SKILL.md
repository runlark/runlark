---
name: runlark-review
description: Use for Runlark CI, pull requests, Sych findings, dependency/tooling updates or delivery; verify the exact current head without leaking private code or credentials.
---

# Current-head review

1. Run `mise exec -- just check` and exercise the actual changed runtime path before pushing. Keep check commands in the justfile; do not establish a second CI-only gate.
2. Use a branch and PR to main. After every push run `scripts/wait-sych [PR_NUMBER]` and inspect both **CI / check** and **Sych / sych**.
3. Confirm the summary is from `sych-reviewer[bot]` and names the current head. Old-head evidence is stale, even if it was green. The waiter re-reads head but its success is only review availability.
4. Read every finding, UNKNOWN and coverage gap. Fix real defects and wait again on the new head. Explain false positives with evidence; do not distort code to satisfy noise. A waiver/risk decision is not proof of safety.
5. Report concrete verification, accepted risks and missing evidence. Never merge with missing current-head review or failing checks.

## CI trust

Use SHA-pinned actions, read-only permissions, no persisted checkout credential and no pull_request_target. Keep Sych waiting in its separate workflow so the CI artifact producer can complete. CI uploads actual Clippy/exit evidence bound to its exact checkout, including a merge commit when applicable; never relabel it as an untested PR head.

The GitHub App and trusted server run outside PR Actions. Private repo access requires both installation selection and the server allowlist/CI selectors. Never put model keys, App keys or production credentials in workflow files, artifacts or chat. Do not weaken reviewer policies, change its image/models/budgets or bypass timeouts to get a clean result.

When upgrading pinned tools/dependencies, run the entire gate and an actual smoke; inspect changed licenses/advisories. Read-only advisory findings do not grant approval or merge authority.
