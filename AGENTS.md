# Runlark agent rules

<!-- AUTONOMY DIRECTIVE — DO NOT REMOVE -->

Execute authorized work to completion. Do not ask for permission on ordinary reversible local
edit/test/verify steps. Preserve others' changes. Use native subagents for independent bounded
work when it materially improves throughput. Escalate only genuine ambiguity, missing authority,
production changes or destructive operations.
<!-- END AUTONOMY DIRECTIVE -->

## Read first

Read [README](README.md), [PRODUCT](PRODUCT.md), [architecture](docs/architecture.md),
[development](docs/development.md), [gates](docs/gates.md) and [ROADMAP](ROADMAP.md).
The [new plan](docs/new-plan.md) defines the reset direction. This repository initially contains
only the plan and development gates. Never describe planned DSH/Rust/SQLite/WASM behavior as implemented.
Legacy history and ADR live in `runlark-lagacy`; do not rewrite or import their obsolete runtime assumptions.

## Skills

Read the matching `.agents/skills/*/SKILL.md` before changing its area:

- `runlark-development`: every implementation, fix, refactor or dependency decision.
- `runlark-review`: CI, GitHub, reviews and delivery. Sych is mandatory for this repo's changes.
- `runlark-rust`: Rust code/contracts/dependencies, pinned stable toolchain and Google Rust style.
- `runlark-workflows`: domain, storage, adapters, ownership, authorization, budgets and approvals.
  Interpret references to old Temporal/PostgreSQL runtime as legacy-specific; new local storage is SQLite.
- `easement`: new files/imports and architecture verification. Before product source, declare actual
  components and run `easement explain`/`propose`; keep strict `--require-evidence` in `just check`.
- `impeccable`: every UI/UX change or review; context once/session, matching playbook and craft floor
  before editing, desktop/mobile rendered exercises and detect before delivery.
- `modern-web-guidance`: search its guides before HTML/CSS/browser JavaScript.
- `feod`: frontend boundaries and public APIs; create the new project's profile before frontend source.
- `reatom`, `reatom-async`: v1001 state, forms, async; `reatom-review` for review and
  `reatom-api-design` before helpers. React/Mantine adapters and props-only presentation stay selected.
  Do not adopt the separate `@reatom/jsx` renderer or migrate routing from generic examples.

[web/DESIGN.md](web/DESIGN.md) is the preserved visual contract. No UI implementation is shipped yet.
No architecture policy is loosened to make a check pass. New declarations must describe the selected
new product and land with evidence, not grant speculative future imports.

## Change flow

1. Work on your branch `codex/...` from fresh `origin/main`; never reset/clean others' work.
2. Read affected contracts/callers. Make the smallest complete change. Prefer existing utilities and
   deletion to new layers. No unused speculative dependencies, adapters, options or fallbacks.
3. Reproduce bugs before fixing. For cleanup write a plan and lock missing behavior with meaningful
   regression tests first; separate writer/reviewer passes. Tests prove observable behavior.
4. Product source must add actual compiler, architecture, behavior and smoke checks in the same PR;
   `scripts/bootstrap-scope.mjs` fails closed until that integration. Do not simply delete the guard.
5. Run `mise exec -- just check` and exercise the real changed path and failures before pushing.
   Conventional Commits are mandatory: `type(scope)!: description` (scope and `!` optional).
   Types: feat, fix, docs, style, refactor, perf, test, build, ci, chore, revert.
   Separate any body/footer from the header with a blank line. Use `revert: ...` for reverts;
   finish fixup/squash work before pushing. Keep PR titles conventional for squash merge.
   Hooks: pre-commit `just lint`, commit-msg validates the message, pre-push validates every
   outgoing unpublished commit before `just check`. Fetch the configured remote before a new
   branch push; already-published history is preserved. Never bypass a failing hook.
6. Open a PR. After every push wait for green `check` and `scripts/wait-sych` for the current head.
   Read all findings, UNKNOWN and coverage gaps; fix defects and explain accepted risks/false positives.
   A summary's availability is not approval. Never merge with missing review or failing checks.
7. Update affected contracts/docs and changelog. Report observed verification, limitations and risks.

Initial empty-repository seed is the sole unavoidable main bootstrap; all substantive changes use PRs.

## Invariants

- Domain is pure. Parse external input and verify scope/rights at boundaries. No external effect
  inside a database transaction or deterministic transition.
- Persist intent before dispatch. Stable command IDs, immutable body, expected revision and durable
  inbox/outbox where needed. Same ID with different payload is a conflict, never a new effect.
- Unknown delivery/outcome/cost remains unknown until reconciled. Cancel requested is not stopped.
- Decisions bind plan/head/base/policy/evidence versions. Acceptance is a human decision.
- One owner per active run; separate worktrees for concurrent writers. Writer-side fencing and
  current authorization at effect boundaries; native developer access is not container isolation.
- Unsafe is forbidden. Rust lints are inherited by all crates, Clippy warnings fail CI, stable 2024
  edition/rustfmt, no broad warning suppressions. Commit lockfiles and check advisories/licenses.
- Every handwritten tracked file stays within 20 000 bytes under the inherited indexed size gate.
  No legacy debt baseline. Generated/vendored exemptions require genuine provenance.
- ADRs, easement policy/baseline and gate debt exceptions change only at the owner's direct request.
  This reset authorizes the new architecture's initial declarations, not later policy bypasses.
- DSH, Wasmtime and external API claims require official, version-bound evidence before use.
- Sych remains standalone. Do not open its DB, embed its engine, or change reviewer image/models/
  budgets/policy to make a review green. App keys and provider credentials stay outside Actions.
- Keep both repositories private. No credentials, personal transcripts or production data in Git.
  No production deployment, paid resource creation or image publication without a request.
- Bootstrap migration preserves old refs, worktrees, ignored data and settings; follow
  [migration](docs/migration.md) and verify both external repository identities.
