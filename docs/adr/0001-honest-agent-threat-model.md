# Honest-agent threat model for the local daemon

Agents run with a shell as the same OS user as the person. We defend against two things:

- **Other origins.**
  - The daemon serves its protocol on a Unix socket that only the user can open.
  - A browser-facing listener, when one exists, requires a capability token and rejects requests that
    carry an Origin header.
  - Until the legacy HTTP API is removed, it drops CORS and allows only loopback Host headers.
- **The agent's own tool surface.** The MCP tools Runlark gives agents cannot accept, request
  changes, answer questions, publish or complete tasks. Every decision carries a typed human actor derived
  from the client channel, never a default string.

We do not defend against a malicious process running as the same user: it can read the token or run the
CLI. Isolating agents (containers or a separate user) is a later decision, and it is required before any
unattended or autonomous mode.

## Considered options

- Strict isolation now: changes the runtime and delays dogfooding.
- Trusting the agent fully: lets an agent accept its own work, which defeats human acceptance.
