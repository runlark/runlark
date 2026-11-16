---
name: runlark-development
description: Use for every Runlark implementation, bug fix, refactor or dependency decision; make the smallest complete change and prove observable behavior.
---

# Verified minimal development

Inspired by [UB-mode](https://github.com/unclebob/ub-mode/blob/master/SKILL.md); this is an original Runlark adaptation, not a vendored upstream skill.

## Make it work

1. Read the requested behavior, current code and every affected caller, contract, fixture and config. Name security/data risks before choosing the change.
2. Reuse the existing pattern or standard library. One decision lives in one place. Subtract dead weight before adding code; no abstraction for hypothetical callers. Run `easement explain <path>` before placing a new file and `easement propose <file> <target>` before an import across components; fix a violation by its `because`, never by editing `easement.yml`.
3. For a bug, reproduce the real failure first. Add the smallest failing behavioral test where honest and practical; see it fail for the intended reason.
4. Implement the whole behavior, including errors and boundary validation. Keep illegal states unrepresentable where a simple type helps. No fake integrations, placeholders or hidden fallbacks.

## Make it right

- Prefer pure domain functions. Parse outside data once; preserve explicit errors at I/O boundaries.
- Do not share writable state when independent ownership works. External operations need stable identities and reconciliation, not blind retries.
- Tests check consumer-visible outcomes, boundaries and transitions, not source text, private helper calls or mocked forwarding. If an honest test is impractical, exercise the real path and report the limit.
- Refactor only the touched responsibility. Keep existing error handling and validation. Do not silence a warning instead of removing its cause.

## Follow practice

Run `mise exec -- just check` and exercise the changed program path. Update contracts, documentation and changelog. Keep commits coherent; push/open/merge only within the owner's task. Follow the review skill before delivery. Report the change, actual verification and risks; never claim unobserved success.

Do not introduce crapper, dryer or mutator commands from upstream: they are not this repository's toolchain. Use its existing strict Clippy, tests, cargo-machete and review gate instead.
