# FEOD — structure

## Where to put an entity

Entities move between layers (common → module and back) — expected.

```text
Need availability without import?            → global (.d.ts, polyfill, shim only)
Tied to a business domain?                   → modules
  Needs a URL?                               → modules/<Name>/pages/
  Only for one top-level page?               → pages/_<page>/ (prefer module pages)
  Too large / needs isolation?               → modules/<Name>/modules/<Sub>/
  Depends on another module?                 → IoC (interface + app/init)
Universal, 1 file, no business logic?        → common
  Several related files / design system?     → modules (not common)
  Needs another module?                      → modules or IoC
Bootstrap / config / router / layout?        → app (layouts may be a module)
URL without domain (404, about)?             → pages (thin)
Everything maps to module pages?             → top-level pages/ may be omitted
```

| Entity                                               | Layer            |
| ---------------------------------------------------- | ---------------- |
| Entry, router, layouts, integrations                 | app              |
| 404, about                                           | pages            |
| URL + domain logic                                   | modules/…/pages/ |
| Feature, API, domain store                           | modules          |
| Design system / UI library                           | modules          |
| One button, formatDate, framework-agnostic utilities | common           |
| env.d.ts, ambient global types                       | global           |

## Base tree (SPA)

```text
src/
  app/            entry, router, layouts/, config/, integrations/, ui/ (layout pieces), styles/, assets/
  pages/          HomePage.tsx, NotFoundPage.tsx, _pageName/ (private module)
  modules/
    feature-name/
      model/              hooks, state, domain helpers and wire-to-client mappers
      containers/         smart components
      presentational/     props/slots-only UI
      pages/              optional thin module pages
      modules/            optional nested modules
      index.ts            public API
  common/         ui/ (primitives), config/, libs/<lib>/ (shared utilities)
  global/         ambient declarations, connected via tsconfig
```

Layer names come from the profile. Top-level `pages/` is optional. FEOD does not
prescribe module internals — only `index.ts` and optionally `modules/`, `pages/`.

This project's React profile uses named arrow-function components in `.tsx`,
hooks in `model/use-<name>.ts` and separately declared props interfaces. One component
per file except compound components. FEOD itself does not prescribe these internals.
When a meta-framework owns routing, keep its required entry and delegate domain
behavior to modules instead of inventing a parallel router.

## Variant trees

Commonless: no `common/`. Common-module: `modules/common/index.ts`.
Multiapp:

```text
src/
  apps/{web,admin}/
  modules/   common/   pages/   (shared)
```

## File-based routing (convention, not required)

| URL           | File                                              |
| ------------- | ------------------------------------------------- |
| `/`           | `pages/index.tsx`                                 |
| `/about`      | `pages/about.tsx`                                 |
| `/products/`  | `pages/products/index.tsx`                        |
| `/posts/123`  | `pages/posts/[id].tsx`                            |
| `/posts/1_2`  | `pages/posts/[userId]_[postId].tsx`               |
| `/docs/a/b/c` | `pages/docs/[...path].tsx`                        |
| named views   | `pages/login.form.tsx`, `pages/login.content.tsx` |
| group w/o URL | `pages/(group)/`                                  |

A manual router in app is equally valid.

## Extensions

SSR — split client/server entry: `app/entry.client.ts` + `app/server/entry.server.ts`
(or a separate server layer, see `docs/base/extensions.md`). Ready-made SSR
frameworks need not follow this.

DDD inside a module:

```text
ModuleName/
  domain/entities/
  adapters/        primary: UI; secondary: API/repos
  index.ts
```

## Migrating to FEOD

1. Carve out `app/` — entry, router, config
2. Pages → `pages/` or `modules/*/pages/`
3. Features → `modules/<Feature>/` with `index.ts`
4. Shared utilities → `common/`
5. Global types/shim → `global/`
6. Fix imports to match the matrix
7. Add feod config and the lint plugin ([lint.md](lint.md)); adopt gradually via `docs/linting/migration.md`
