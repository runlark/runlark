# DSH Integration Spike

This document formalizes the technical investigation for Milestone 2: DSH Integration Spike,
addressing the integration requirements and critical risks defined in [new-plan.md](new-plan.md) and
[architecture.md](architecture.md). It establishes the pinned upstream repository and commit, audits
supported runtime APIs, maps managed session operations to the Rust command layer, and specifies
the upstream patch boundary.

---

## 1. Repository & Commit Pinning

The upstream project evaluated and selected for Runlark is **DeepSeek Harness** (`dsh`), developed
by DeepSeek AI. It provides an extensible agent framework built on the Cordis microkernel,
combining a headless agent loop with a React-based host UI.

### 1.1 Pinned Metadata & Provenance

| Property                | Value                                                             |
| ----------------------- | ----------------------------------------------------------------- |
| **Upstream Project**    | DeepSeek Harness (`deepseek-ai/deepseek-harness`)                 |
| **Repository URL**      | `https://github.com/deepseek-ai/deepseek-harness.git`             |
| **Pinned Release Tag**  | `dsh-v0.2.2-rc.3`                                                 |
| **Pinned Commit SHA**   | `38a6a5c965aa99ab525161efb7cc5cf1fe62fced`                        |
| **Master Sync Commit**  | `39163dd30f703a79079e2d2446545cff3c453738`                        |
| **Release Tag Date**    | 2026-11-13 13:26:07 +0800 (2026-11-13T05:26:07Z)                  |
| **npm Packages**        | `@deepseek-ai/dsh@0.2.2-rc.3`, `@deepseek-ai/dsh-root@0.2.2-rc.3` |
| **License**             | MIT License (Copyright (c) 2026 DeepSeek)                         |
| **Runtime & Toolchain** | Node.js (>=22), TypeScript (strict), pnpm monorepo, Cordis v4.1.0 |

### 1.2 Selection Rationale

- **Disambiguation**: Alternative projects referencing "DSH" (e.g. Daydreams) lack desktop/web agent
  host infrastructure, session admission, and UI slot systems. DeepSeek Harness directly implements
  the host bridge, UI slots, session controller, and agent loop required by Runlark.
- **Version Evaluation**: The `0.1.x` series is deprecated and lacks pluggable session admission.
  Candidate `0.2.1-rc.5` was superseded by `0.2.2-rc.1` (npm latest) and `0.2.2-rc.3` (npm next).
  Release candidate `0.2.2-rc.3` includes critical fixes for multi-worktree synchronization, browser
  session authentication, and projection reconciliation.
- **Commit Pinning**: Commit `38a6a5c965aa99ab525161efb7cc5cf1fe62fced` marks the exact immutable
  release tag `dsh-v0.2.2-rc.3`. Master sync commit `39163dd30f703a79079e2d2446545cff3c453738`
  (PR #8422) syncs this tag into `master`. All Runlark adapters pin to this revision.

---

## 2. Supported DSH APIs & Integration Architecture

The DSH runtime is organized as a Cordis microkernel with pluggable service contracts. The following
core subsystems were audited in the pinned source code:

### 2.1 Host Bridge Mechanics

- **Unary RPC**: HTTP POST addressed to `/api/<channel>/<endpoint>`. Requests carry typed
  `ClientRequest` envelopes with `rpcId`, method name, and JSON payload. Responses return `{ ok:
  true, value, rpcId }` or multipart form data for binary attachments.
- **Multiplexed Streaming**: WebSocket endpoint `/api/remote.mux` handled by
  `RemoteStreamMuxServer`. Multiplexes logical stream channels (`session.follow`, `session.control`)
  over a single authenticated connection.
- **Application Event Stream (`$events`)**: Registered via
  `ctx.typertGateway.registerRemoteEvents()`. Initiates with a `{ type: 'ready', clientId, host }`
  handshake frame followed by continuous event dispatch.
- **Transport Abstraction**: Decoupled via `ClientTransportHooks`:

  ```ts
  export interface ClientTransportHooks {
    rpc?: ClientConnectionRpc;
    fetch?: RpcFetch;
    openStream?: RpcStreamOpen;
    loadBundle?(url: string): Promise<void>;
  }
  ```

  Enables routing client traffic in-process or redirecting to the Runlark local daemon.

### 2.2 UI Mounting Points & Framework Integration

- **Host UI Engine**: DSH client UI is built on React 18/19 using `@deepseek-ai/dsh-client-ui-slots`
  and `@deepseek-ai/dsh-client-ui-renderer`.
- **Slot Composition**: Supports typed slots across `single`, `list`, `keyed`, `chain`, and `hoc`.
- **Mounting Seams for Runlark Views**:
  - **Docking Panels (`ui-dockkit`)**: Houses Runlark's Kanban board, diff viewer, and decision
    inbox as docked or floating panels.
  - **Right Sidebar (`ui-sidebar-right`)**: `ctx.sidebarRightTabs` registers custom tabs for active
    task cards, run metadata, and budget tracking.
  - **Tool Cards (`tool.call.toolview`)**: Keyed slots render custom UI cards for Runlark tool
    invocations (decisions, approvals, evidence snapshots).
- **Framework Compatibility**: DSH client does not bundle Mantine. Runlark packages its UI as a DSH
  client plugin (`packages/dsh-plugin`), loading Mantine style tokens scoped to Runlark container
  roots, preserving the visual contracts defined in [DESIGN.md](../web/DESIGN.md).

### 2.3 Custom Tool Registration & Execution Pipeline

- **Definition API**: Registered via `ctx.tools.register(defineTool({ ... }))` globally or scoped to
  an individual agent on `agent.ctx.tools`. Tools declare typed parameter schemas, output renderers,
  and execute functions. `exec.signal` provides an `AbortSignal` for cooperative cancellation.
- **Lifecycle & Guards**: Supports `tools/pre-execute` waterfall policies (`allow`, `deny`, `ask`),
  synchronous monotonic denial via `ctx.tools.guard()`, and execution timeouts.

### 2.4 Managed Session Admission & Lifecycle

- **Session Creation**: Endpoint `@Remote('create')` accepts `SessionCreateRequest` with
  `sessionId`, `cwd`, and `agentPreset`.
- **Single-Writer Lock**: Enforced by `SessionAlreadyOwnedError` if another writer actively owns the
  session lease.
- **Cold Agent Resolution**: `ctx.agents.resume({ resumeSessionId, setup })` deduplicates concurrent
  resumptions via an in-flight Promise map and restores history from durable session storage.

### 2.5 Working Directory & Permissions Boundary

- **CWD Invariant**: Bound permanently to `SessionHeader.cwd` upon session initialization.
  Cross-directory reuse or switching is rejected with `ApiSessionCwdConflict`.
- **Filesystem Confinement (`dsh-fs-sandbox`)**: In `workspace-write` mode, file operations
  (`writeText`, `editText`, `patchText`, `deleteFile`, `moveFile`) enforce strict canonicalization
  against `session.header.cwd` and platform temp dirs. Path escapes throw `FS_SANDBOX_DENIED`.
- **Process Sandbox (`dsh-sandbox`)**: Restricts child process execution using platform primitives:
  macOS Apple Seatbelt (`sandbox-exec`), Linux Bubblewrap (`bwrap`) + Landlock, Windows restricted
  tokens + ACLs.

### 2.6 Event Streaming & Interaction

- **Event Follow Stream (`session.follow`)**: Streams append-only `SessionWireEvent` records
  (`turn/start`, `user/message`, `assistant/message`, `tool/call`, `tool/result`, `turn/end`).
- **Live Assistant Chunks**: `assistant/live-chunk` provides ephemeral streaming token feedback.
- **User Questions & Approvals**: Durable questions tool (`ask_user_question`) pauses the agent
  turn until human response is received via `@Remote('answer')`. Answers survive restarts.

### 2.7 Cancellation & Stop Mechanics

- **Cancellation Endpoint**: `@Remote('cancel')` calls
  `agent.cancel({ kind: 'user' }, { keepInbox: true })`.
- **Abortion Cascade**: Triggers `this.phase.abort.abort(cause)`, firing `AbortSignal` on the active
  LLM stream and in-flight tools. Streamed partial attempts are committed cleanly; queued inbox items
  remain intact for subsequent turns.

---

## 3. Rust Command Layer Operations & Critical Integration Risk

### 3.1 The Critical Risk: Direct In-Process Bypass in Native DSH

Per [new-plan.md](new-plan.md) lines 28–31, all core operations (`Send`, `Stop`, `resume`, `cwd`,
`archive`) of a managed session must pass through Rust:
$$\text{Action} \longrightarrow \text{Persisted Command (SQLite)} \longrightarrow \text{Forwarded Effect (DSH)}$$

Auditing the native DSH controller revealed a fundamental architecture risk:

1. **Direct In-Process Call**: In `packages/api/session-controller/src/commands.ts`, methods
   `prompt()` and `cancel()` invoke `agent.followup()` and `agent.cancel()` directly in Node.js
   memory without persisting command intents or checking revision fencing.
2. **RPC Interceptor Limitation**: In `@deepseek-ai/dsh-client-connection/src/rpc-host.ts:293-295`,
   the `/api` channel permits only **one** interceptor. The API Gateway already claims `/api`,
   preventing plugins from registering an external RPC pre-filter.
3. **Event Hook Inadequacy**: Hook `agent/inbox/inserted` is a fire-and-forget `emit` event that
   cannot pause or divert dispatch. Hook `agent/pre-step` fires after the turn has already started,
   after history is committed, and does not cover `cancel()`.

Therefore, stock DSH cannot guarantee persist-before-send without an intercepted command seam.

### 3.2 Mapping the 5 Managed Operations Through Rust

| Operation   | User Action                         | Rust Command Layer                                                                             | Forwarded DSH Effect                                                        | Crash / Restart Reconciliation                                                                 |
| ----------- | ----------------------------------- | ---------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------- |
| **Send**    | User enters prompt and submits      | Persist `Command::SendPrompt` in SQLite outbox with stable `command_id` and expected revision. | Interceptor forwards payload to `agent.followup()`.                         | Replay unconfirmed outbox commands. Deterministic `command_id` deduplicates processing.        |
| **Stop**    | User clicks Stop button             | Persist `Command::CancelRun` in SQLite. Transition run status to `CancelRequested`.            | Interceptor calls `agent.cancel({ kind: 'user' }, { keepInbox: true })`.    | Status remains `CancelRequested` until confirmed `turn/end` event reconciles run to `Stopped`. |
| **Resume**  | Daemon restart or browser reconnect | Verify run lease, worktree state, and SQLite outbox.                                           | Call `ctx.agents.resume()` and re-attach `session.follow` stream.           | Resumes stream from last confirmed sequence number without launching duplicate runs.           |
| **CWD**     | Run initialization                  | Create isolated Git worktree at `$WORKSPACE/.runlark/worktrees/<run_id>`.                      | Initialize session with `cwd: worktree_path`. Confine via `dsh-fs-sandbox`. | Verify directory matches lease. Rejects mismatch with `ApiSessionCwdConflict`.                 |
| **Archive** | Run completed or accepted           | Persist `Command::ArchiveRun`. Extract diff and evidence into SQLite/CAS.                      | Call `workspaceRegistry.archiveSession(sessionId, { stopActivity: true })`. | `ArchivedSessionGate` rejects further execution. Worktree retained or cleaned per policy.      |

---

## 4. Upstream Patch Specification

To eliminate the in-process bypass while avoiding an invasive fork, a minimal, isolated patch is
specified for `@deepseek-ai/dsh-api-session-controller`.

### 4.1 Boundary Encapsulation & Interceptor Interface

The patch introduces `SessionCommandInterceptor` in
`packages/api/session-controller/src/commands.ts`. Unmanaged sessions bypass the interceptor and
execute native logic without behavioral changes (adding only a fast `handles()` check).

```ts
import type { SessionId, MessageSource } from "@deepseek-ai/dsh-session";
import type {
  SessionPromptRequest,
  SessionPromptValue,
  SessionCancelRequest,
  SessionCancelValue,
} from "./types.ts";

export interface SessionCommandInterceptor {
  /** Returns true if this session is managed by Runlark. */
  handles(sessionId: SessionId): boolean;

  /** Intercept prompt submission to persist intent before execution. */
  prompt?(
    request: SessionPromptRequest,
    source: MessageSource,
    next: () => Promise<SessionPromptValue>,
  ): Promise<SessionPromptValue>;

  /** Intercept cancellation to record CancelRequested before halting. */
  cancel?(
    request: SessionCancelRequest,
    next: () => SessionCancelValue,
  ): Promise<SessionCancelValue> | SessionCancelValue;
}

declare module "@deepseek-ai/cordis" {
  interface Context {
    sessionCommandInterceptor?: SessionCommandInterceptor;
  }
}
```

### 4.2 Upstream Code Modification

The patch in `SessionCommandController` wraps `prompt()` and `cancel()` in ~25 lines of code:

```diff
--- a/packages/api/session-controller/src/commands.ts
+++ b/packages/api/session-controller/src/commands.ts
@@ -321,6 +321,11 @@ export class SessionCommandController {
   async prompt(request: SessionPromptRequest, source: MessageSource): Promise<SessionPromptValue> {
+    const interceptor = this.ctx.get('sessionCommandInterceptor')
+    if (interceptor?.handles(request.sessionId) && interceptor.prompt) {
+      return interceptor.prompt(request, source, () => this.executePrompt(request, source))
+    }
+    return this.executePrompt(request, source)
+  }
+
+  private async executePrompt(request: SessionPromptRequest, source: MessageSource): Promise<SessionPromptValue> {
     const agent = await this.resolveAgent(request.sessionId)
     // ... existing native prompt execution ...
   }

@@ -526,6 +531,11 @@ export class SessionCommandController {
   cancel(request: SessionCancelRequest): SessionCancelValue | Promise<SessionCancelValue> {
+    const interceptor = this.ctx.get('sessionCommandInterceptor')
+    if (interceptor?.handles(request.sessionId) && interceptor.cancel) {
+      return interceptor.cancel(request, () => this.executeCancel(request))
+    }
+    return this.executeCancel(request)
+  }
+
+  private executeCancel(request: SessionCancelRequest): SessionCancelValue {
     const agent = this.ctx.agents.get(request.sessionId)
     // ... existing native cancel execution ...
   }
```

### 4.3 Contract Testing Strategy

Contract tests in `packages/dsh-plugin/test/interceptor.test.ts` lock the interface and guard
against upstream drift across 5 core invariant cases:

1. **Native Unmanaged Regression**: Unmanaged sessions (`handles() === false`) invoke stock
   `executePrompt` and `executeCancel` without delay or side effects.
2. **Send Persist-Before-Dispatch**: Dispatched `session.prompt` invokes `interceptor.prompt()`.
   Verifies that the command is persisted in SQLite outbox before `next()` is called.
3. **Stop Invariant (_Cancel requested is not stopped_)**: Dispatched `session.cancel` transitions
   status to `CancelRequested`, signals `agent.cancel()`, and transitions to `Stopped` only when
   confirmed by a `turn/end` event.
4. **CWD Worktree Confinement**: Sessions initialized with a dedicated worktree path enforce
   filesystem isolation. Path escapes are trapped with `FS_SANDBOX_DENIED`.
5. **Post-Archive Invariant**: Archiving a session halts running activity and engages
   `ArchivedSessionGate`, blocking subsequent `agent/pre-step` invocations.

---

## 5. Milestone 3 (A0 Slice) Delivery Plan

With Milestone 2 technical risks retired, Milestone 3 will deliver the complete A0 vertical slice:

1. **Architecture Boundaries**: Declare components in `easement.yml`, run `easement explain`/
   `propose`, and enforce `easement check --require-evidence` in `just check`.
2. **Quality Gates & Toolchain**: Connect Rust workspace (Clippy, rustfmt, cargo-deny,
   cargo-machete), SQLite migration/reconciliation tests, and DSH plugin checks.
3. **Pure Domain (`crates/runlark-domain`)**: Pure models for Task, Run, Command, Event, and
   Execution State with zero I/O dependencies.
4. **Local Storage & Outbox (`crates/runlark-local`)**: SQLite schema, persist-before-send outbox,
   idempotent execution, and restart reconciliation.
5. **Local Daemon (`crates/runlark-cli`)**: CLI and HTTP/IPC daemon process orchestrating session
   leases and worktrees.
6. **DSH Bridge Adapter (`packages/dsh-plugin`)**: Implements `SessionCommandInterceptor`, forwards
   commands to daemon endpoints, and mounts Runlark's UI views into DSH dock panels.
