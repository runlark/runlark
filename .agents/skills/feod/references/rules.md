# FEOD — rules

Single source for import legality and variants. The project profile
(`feod.profile.md`) selects which deltas below apply.

## Cross-level import matrix (base)

| Imports ↓ / from → | global | common | modules         | pages | app |
| ------------------ | ------ | ------ | --------------- | ----- | --- |
| **global**         | —      | ❌     | ❌              | ❌    | ❌  |
| **common**         | ✅*    | ✅     | ❌**            | ❌    | ❌  |
| **modules**        | ✅*    | ✅     | ✅ (public API) | ❌    | ❌  |
| **pages**          | ✅*    | ✅     | ✅ (public API) | ❌    | ❌  |
| **app**            | ✅*    | ✅     | ✅ (public API) | ✅    | ❌  |

\* global is not imported — wired via tsconfig / entry init

\*\* common → modules only via IoC (interface in common/module, implementation in app)

npm packages follow the file-specific guard policy; presentation does not own hooks or I/O.

## Path rules

| Import type        | Path                                                                      |
| ------------------ | ------------------------------------------------------------------------- |
| Cross-level        | `@/common/libs/dates/format-date`                                         |
| Cross-module       | `@/modules/User` (→ index.ts)                                             |
| In-level (pages)   | `./_home` (private module)                                                |
| In-module          | `../model/use-users` from a container                                     |
| Submodule → parent | Inject a required interface/callback; do not deep-import parent internals |

Submodule from outside: public API of the parent only.

## Nested levels

The path defines the logical layer, not folder depth. Special rules only for:

| Path                                                 | Logical layer                 |
| ---------------------------------------------------- | ----------------------------- |
| `modules/<Name>/modules/<Sub>/`                      | modules (submodule)           |
| `modules/<Name>/pages/`                              | pages (module pages)          |
| `pages/_name/`                                       | modules (private page module) |
| `modules/<group>/<child>/` with `<group>/_/index.ts` | modules (transit submodule)   |

Everything else (`common/ui/Button.tsx`, ...) belongs to the enclosing layer.
Rule priority: project profile > level rules > base rules.

- **Module pages** — a module may import its own pages to re-export through the public API.
  Preferred over top-level `pages/` for domain URLs.
- **Private page module** (`pages/_name/`) — usable only by the owning page; has no pages
  of its own; does not import other entities at pages level. Use sparingly.
- **Transit module** — container without its own `modules/`; `_/index.ts` is the private
  API for its submodules. Lint option: `transitModules`.

```text
modules/package-manager/
  _/index.ts
  npm/
  yarn/
  pnpm/
```

## Modifications (deltas to base)

| ID              | Delta                                                                                                                                                                                                |
| --------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `base`          | default; nothing changes                                                                                                                                                                             |
| `commonless`    | no `common/`; everything in `modules/` grouped by meaning (`modules/ui/`, not `utils/`). Only allowed types-only module: utility types (`Flatten<T>`). Prefer submodules over new top-level modules. |
| `common-module` | `modules/common/index.ts` replaces `common/`; minimize its imports of other modules; keep extracting entities into modules as the project grows.                                                     |
| `multiapp`      | `apps/<name>/` replaces the single `app/`; modules/common/pages shared. Combines with the three above. Each app has the `app` rules.                                                                 |

Alternative layer names keep the same rules: `boot`≈app, `views`/`routes`≈pages,
`shared`≈common (FEOD semantics, not FSD). Names are set in the profile / `layerDirs`.

## Anti-patterns

- ❌ Importing module internals (`@/modules/User/api/userApi`) — only `@/modules/User`
- ❌ `common/index.ts` or a barrel in common (unless the profile allows it)
- ❌ Business logic in pages or app
- ❌ A module "for one utility" — use common or a submodule
- ❌ Pseudo-modules `utils`, `types`, `constants` — group by meaning
- ❌ More than ~3 levels of submodule nesting without justification
- ❌ Direct module → module imports instead of IoC when coupling is strong
- ❌ Moving a design system into common unless the profile explicitly chooses `common/ui`
- ❌ Zigzag imports between modules

## FSD habits to unlearn

| Habit                             | In FEOD                         |
| --------------------------------- | ------------------------------- |
| common = shared abstraction layer | common = temporary staging area |
| widgets/features/entities         | one modules layer               |
| barrel in shared/common           | `index.ts` only in modules      |
| logic in pages                    | modules + thin pages            |
| utils/types/constants modules     | common or domain modules        |

## IoC pattern

Use a required interface or callback supplied by app composition. Keep the interface
in the owning module or common when it is domain-neutral; put integration behavior
in app. Missing dependencies must not silently become console logging, no-ops or fake
implementations. Pass display data and event callbacks down through props.

React module example — [templates/react-spa.md](templates/react-spa.md).
