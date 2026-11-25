# The board UI is a native GPUI client

The board, the task view, acceptance and the agent stream are built as a native desktop client on GPUI,
in the spirit of Ghostex. It is a protocol client like any other.

- **Design:** layout, density and interaction patterns follow Ghostex (session sidebar, chat and terminal
  panes, changes view, plan review). The stage timeline and the "waiting for you" group follow PurpleFlow.
  Runlark's brand tokens stay.
- **Retired code:** the React/Mantine `web/` code and its frontend gates are retired.
- **Until the client exists:** dogfooding uses the CLI.

## Considered options

- Keep the React/Mantine web UI: reuses code, but the owner wants a native Ghostex-like client.
- Fork Ghostex: about 1.5–2M lines (estimated), effectively one maintainer, daily releases; we borrow its
  UX, not its code.
