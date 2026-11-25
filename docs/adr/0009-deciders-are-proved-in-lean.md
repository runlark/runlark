# Deciders are written to be proved in Lean

Runlark's correctness rests on a few invariants:

- at most the configured number of runs execute at once;
- a task completes only through an accept decision;
- cancel requested never becomes stopped without confirmation.

We prove these over the real code instead of a separate model. The deciders are extracted to Lean 4 with
Aeneas, and a mutation check shows that each proof fails when the guarded condition is changed.

Extraction only works on a restricted subset of Rust, so the decider kernel is written in that subset from
the start:

- pure, synchronous functions;
- no I/O, no trait objects, no interior mutability;
- only `std` and `core`, with no external crates.

Encoding, hashing and storage live outside the kernel and hand the kernel plain values. The proof job
starts as a separate, non-blocking CI job and becomes required once it is stable.

## Considered options

- A hand-written Lean model with equivalence tests (the legacy approach): the model and the code drift
  apart.
- Property tests only: these sample behaviour but do not prove the invariants.
