# Interactive board first, autonomous factory later

Runlark is one product whose core is reliable execution and human acceptance. The interactive board,
where a person shapes tasks, answers questions and accepts results, is the primary mode and the first to
be dogfooded. The autonomous "factory" mode comes later as an extension of the same core:

- scheduled connectors bring work in;
- modules run unattended in sandboxes;
- people accept the results in the morning.

The collaboration server is kept but not extended until the board passes its dogfooding week.

## Considered options

- Factory as the primary mode: it depends on unattended isolation and on intake contracts that do not
  exist yet.
- Two separate products: would duplicate the execution and acceptance core.
