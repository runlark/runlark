---
name: runlark-rust
description: Use when writing or reviewing Runlark Rust code, Cargo dependencies or public contracts; apply Google Rust Style Guide with the pinned stable toolchain.
---

# Rust style and contracts

Authority: [Google Rust Style Guide](https://google.github.io/styleguide/rustguide.html), then public Rust style/API guidelines. This is application code: favor clear local solutions over reusable frameworks. Library API rules become binding for contracts shared with other teams.

## Automated rules

- Use the pinned toolchain, Rust 2024, 100 columns and stable rustfmt. Every crate inherits `[lints] workspace = true` and `publish.workspace = true`.
- Unsafe is forbidden; ignored must-use results are denied. All/pedantic Clippy warnings fail the gate; wildcard imports, absolute type paths and unwraps are flagged.
- Run `just lint-rust`, `just test`, `just docs` and ultimately `just check`. Use `just style-fix` for sych's blank-line-after-if convention.
- Keep used dependencies in `workspace.dependencies`; no unused SQLx/Temporal/React packages, wildcard versions or untrusted Git sources. Commit Cargo.lock.

## Review rules not fully expressed by stable tooling

- Import items from one module per use statement. Import types unqualified; call free functions as `module::function`. Prefer local trait imports when used in one function. No custom preludes or glob imports except Google's explicit local exceptions.
- Leaf files hold implementations and are named after their primary item. Non-leaf modules declare/re-export; small entry-point composition is allowed. Split crates for real responsibility boundaries, not one type per crate.
- Prefer unique ownership and borrowing over clone/Rc/Arc. Use `&str`, `&[T]` and `Option<&T>` where appropriate; avoid unnecessary allocations/copies. Shared pointers require a real ownership need.
- Prefer named fields and meaningful newtypes when values can be confused. Use enums instead of ambiguous booleans. Implement standard traits only when useful; derive when possible.
- Use Option for obvious absence, Result with context for recoverable failures, and expect with an invariant explanation only where panic is justified. Prefer pattern matching over check-then-unwrap. Do not catch panics as normal error handling.
- Document public behavior, errors and invariants, not obvious mechanics. Apply must_use/non_exhaustive to genuinely shared evolving APIs as appropriate; do not generalize private application code unnecessarily.
- Match all known variants and retain explicit unknown/error states. Prefer clear loops over tangled iterator chains. No unstable features or code executed before main.

Chromium-only no-async/Bazel layout/import rules and Fuchsia mutex requirements do not apply. Tokio/Axum and conventional Cargo layouts are deliberate. Unstable rustfmt import-granularity options are not enabled; review those rules manually instead of adding nightly.
