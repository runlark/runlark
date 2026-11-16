# FEOD — workflows

## Add a module

1. Create `src/modules/<name>/`; use `model/`, `containers/` and `presentational/` per the profile.
2. Export **only what is needed** from `index.ts`; internal files are never imported from outside.
3. Growth: `modules/<Name>/modules/<Sub>/`; not deeper than ~3 levels without reason.
4. Needed by one page only: prefer `modules/<Name>/pages/`; `pages/_<page>/` sparingly.
5. Cross-module dependency: IoC — interface in the module, implementation in `app/integrations/`, wiring in `app/init/`.
6. Update affected existing module docs and contracts; create a module README only when requested.
7. Unit tests live inside the module; multi-module integration tests in `app` (or the parent module).

```ts
// index.ts template
export { FeatureList } from "./containers/FeatureList";
export type { Feature } from "./model/feature";
```

## Add a page

1. Preferred for domain URLs: `modules/<name>/pages/<RoutePage>.tsx`, exported through its module API.
2. Otherwise `src/pages/` (file-based convention: [structure.md](structure.md)).
3. A page is **thin**: composes things from `@/modules/<Name>`; no business logic. It is
   not merely a re-export of one module component (see the BAD example in the template).
4. Register routes in app composition using the project's chosen router; add no router dependency just for a template.
5. Shared route constants belong in `common/config/routes.ts`.

## New project

```text
- [ ] 1. Run init.md (or accept base defaults) → feod.profile.md
- [ ] 2. Create src/ layers per the profile
- [ ] 3. Alias @ → src in bundler and tsconfig paths
- [ ] 4. Real app bootstrap in app/main.tsx and app composition only when a UI is requested
- [ ] 5. Wire the ESLint/Oxlint plugin (lint.md); feod config only if the installed version reads it
- [ ] 6. Split features into modules; pages stay thin; shared bits go to common
```

Vite resolves the same `@/*` paths as TypeScript via `resolve: { tsconfigPaths: true }`.

## Checklist before finishing

- [ ] Imports follow the layer matrix and the profile
- [ ] Modules export only through `index.ts`
- [ ] Pages are thin; logic lives in modules
- [ ] Common has no barrel (unless the profile allows it); entities are usually one file
- [ ] Global only for shim/polyfill/global types
- [ ] App has no business logic
- [ ] Profile and supported lint configuration agree
- [ ] One component per file except compound components; named props interfaces outside parameters
- [ ] Requested ESLint/Oxlint integration covers the intended source files
- [ ] Relevant lint checks pass; rule fixes have a regression reproducer
