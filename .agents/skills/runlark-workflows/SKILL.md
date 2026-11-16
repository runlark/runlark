---
name: runlark-workflows
description: Use for Runlark domain, read models, local durable runs, SQLite, optional collaboration, DSH adapters, runner ownership, authorization, budgets or approvals.
---

# Durable workflow boundaries

Read [architecture](../../../docs/architecture.md) and [the new plan](../../../docs/new-plan.md).
These are obligations for new product code, not claims that those subsystems already exist.
Legacy ADRs, Temporal and the PostgreSQL engine remain in the separate legacy repository.

## Facts and views

Trace a new screen field: field → view → recorded fact → the command or operation that caused it.
A view folds recorded facts; where it would have to infer what happened, record the fact and
update the model in the same change. Snapshot state and its event cursor must agree.

## Ownership

Rust daemon owns Task/Run/Step/Attempt, commands, decisions, queue, limits and recovery.
SQLite owns local durable state, journal and outbox. DSH owns conversations, models, tools and
agent sessions; it does not independently accept tasks. Rust owns worktrees and writer admission.
Long waits survive without a live DSH Promise or WASM instance. In team mode the server orders
shared commands and assigns one runner; an offline client does not become shared authority.
Sych owns reviews/evidence and its internal ledger. UI projections never schedule independently.

## Before an external effect

1. Authenticate and validate workspace scope and current permission; do not trust historic authorization.
2. Persist the intent with a durable business_operation_id and outbox entry in one transaction; finish the transaction before network I/O.
3. Preserve operation identity across delivery retries and restart. A lost response is unknown, not a fresh semantic attempt. Resolve it through provider lookup/reconciliation.
4. Enforce generation/fencing at the actual writer, with one writable checkout owner. Deduplicate verified incoming events in an inbox.

Pure transitions have no SQL, HTTP, Git, wall-clock, environment, filesystem or model effects.
Persist effects before dispatch. Same command ID and immutable body returns the previous result;
a changed body with the same ID is a conflict. Send/Stop/resume/cwd/archive of managed sessions
must pass the Rust command boundary. Replaying local events does not make external effects idempotent.

## Acceptance and recovery

- Keep execution, review, evidence completeness and product acceptance separate.
- Bind approval/merge to exact head/base/plan/policy/evidence. Re-check head and authorization immediately before effect.
- Cancel requested is not stopped. Confirm stop/finish, clean resources, then release capacity. Unknown outcomes remain visible.
- Separate known spend, held reservation and unknown charge; Sych child settlement is imported once, not added to parent totals twice.
- Pin task revision, Git base and workflow/package digest/config in the run. Keep packages needed
  by unfinished runs. Use expand/contract migrations and test restart, lost ACK, duplicate events,
  revoked permissions, unreachable-runner cancellation, stale revisions and version overlap.
- Separate worktrees for concurrent writers. One run has one owner; do not automatically transfer
  an active run to another machine on lost connectivity. Shared start/accept/publish wait for the server.
- Native developer credentials are not isolation. WASM capability limits do not constrain trusted
  Node/TS host code or UI bundles. Test the actual execution profile's side-effect boundaries.

Do not create a new workflow type, abstraction, microservice or crate for each customer scenario. Compose existing verified steps when they exist. No feature is complete with a mocked runner, in-memory financial ledger or no-op Sych adapter.
