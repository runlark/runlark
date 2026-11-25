# Board columns are computed, not stored

A task's column is computed from its lifecycle, the state of its current run and any pending decision,
the way Jira and Linear group statuses into categories. Moving a card issues a command:

- Ready → In progress starts a run.
- In progress → Ready requests a stop.
- Acceptance → In progress opens "request changes".
- Moving into Done is refused; the bound accept dialog opens instead.

There is one source of truth, and the board cannot contradict the run.

## Considered options

- Two independent axes, a stored column plus a run stage: they drift apart and need repair rules.
- Stages as columns: the board's shape would depend on the workflow, and repair loops would move cards
  backwards.
