# Tactical DDD, adopted aggregate by aggregate

The domain is modelled tactically:

- Each aggregate is a pure decider: `decide(command, state) → events` and `evolve(state, event) → state`.
- Each aggregate has one repository that saves with an expected revision.
- Each use case has one application service in the daemon.
- Rules that span aggregates live in process managers: admission, the workflow pipeline and sync.
- Domain events are the only output of a transition.

We migrate gradually (strangler):

- Security-boundary fixes land in the current code.
- An invariant fix lands by migrating its aggregate first; Acceptance and Run come first.
- The collaboration server migrates later.

Code that changes state outside a decider (stores, the repair pipeline) is deleted as its aggregate
migrates. `easement.yml` gains the application and process-manager components when they appear.

## Considered options

- Rewrite the solo core first and dogfood afterwards: months without feedback.
- Fix everything in the current structure and introduce DDD after dogfooding: every invariant fix would
  be written twice.
