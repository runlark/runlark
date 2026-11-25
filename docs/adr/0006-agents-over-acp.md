# Agents run behind ACP, quota comes from a separate source

Runlark drives agent harnesses (Codex via `codex-acp`, opencode, omp, and later its own agent) over the
Agent Client Protocol.

- **Tools.** Runlark gives every ACP session the same MCP server, which carries its tools: report, ask a
  question, attach evidence. It is scoped by a per-run token. No per-harness plugin is needed.
- **Quota.** ACP has no quota fields, so a separate quota source reads subscription windows from each
  provider, such as Codex rate limits and opencode usage. Unknown quota stays unknown and is shown.
- **Dedupe.** ACP v1 has no idempotency key, so Runlark's persisted command is the dedupe authority, and
  reconnects reconcile through session replay. We adopt ACP v2 idempotency keys when v2 is stable.
- **Own agent.** Runlark's own ReAct agent ships later as a separate ACP agent binary, so the core speaks
  only ACP.

## Considered options

- Native adapters for the Codex app-server and opencode serve: they expose quota and IDs, but mean two
  experimental protocols to track.
- DSH subagents: one-shot and auto-approving, with no human round-trip.
