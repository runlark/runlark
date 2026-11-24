# Runlark is a CLI and a JSON-RPC server, without DeepSeek Harness

Runlark no longer builds on DeepSeek Harness (DSH). DSH is a developer preview with promised breaking
changes, no extension point for intercepting commands, and no external contributions. Its agent loop also
cannot use the subscriptions we actually run agents on.

Runlark is a CLI plus a long-running daemon, modelled on the Codex app-server:

- **Protocol:** JSON-RPC 2.0, defined in a `runlark-protocol` crate, with requests, notifications and
  server requests (questions and approvals are sent to clients).
- **Transports:** a Unix socket that only the user can open, and a `--stdio` bridge.
- **Schema export:** JSON Schema and TypeScript are generated from the Rust types and checked for drift.

Every UI is a client of this protocol. The REST API and the DSH plugin, patch and dependencies are removed
once the new path runs a task end to end.

## Considered options

- Keep DSH as the UI host and agent loop: needs a patch rebased against a fast-moving preview, and the
  subscription path is blocked.
- Expose only an ACP agent facade: ACP cannot express the board, acceptance bindings or quota.
- Keep REST: no server-to-client requests, and the contracts are hand-written on both sides.
