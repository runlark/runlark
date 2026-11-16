# FEOD — React SPA templates

React 19, TypeScript and Vite. These are reference examples, not an application to
create automatically. The project profile is authoritative; do not install a router,
state manager, UI kit or transport library just to follow this template.

Read the [frontend coding standard](../../../../../web/docs/CODING_STANDART.md):
one component per file except compound components, and a named props `interface`
declared outside the component. Components use named arrow-function exports.

## Layout

```text
src/
  app/
    main.tsx
    App.tsx
    styles/
  pages/
    HomePage.tsx
  modules/
    counter/
      index.ts
      model/
        use-counter.ts
      containers/
        Counter.tsx
      presentational/
        CounterView.tsx
      __tests__/
  common/
    ui/
    config/
    libs/
      numbers/
        format-count.ts
  global/
```

`counter` is only a self-contained illustration of state ownership. Choose real
module boundaries from the task, not from this example. Nested modules and module
pages remain available as described in [structure.md](../structure.md).

## Bootstrap and thin page

### `src/app/main.tsx`

```tsx
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { App } from "./App";

const root = document.querySelector("#root");

if (root === null) {
  throw new Error("Missing root element");
}

createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
```

The HTML entry must provide `id="root"` when an actual app is implemented. Do not
add an entrypoint or an empty HTML shell while the project intentionally has no UI.
Global styles belong to `app/styles/` and are imported by the real bootstrap.

### `src/app/App.tsx`

```tsx
import { HomePage } from "@/pages/HomePage";

export const App = () => <HomePage />;
```

Routing, providers and layout are composed in `app`. Use the project's chosen
router only when the task needs routing; this example needs no router dependency.

### `src/pages/HomePage.tsx`

```tsx
import { Counter } from "@/modules/counter";

export const HomePage = () => (
  <main>
    <h1>Counter</h1>
    <Counter />
  </main>
);
```

Pages compose UI and route parameters. State transitions, requests, hooks and
wire-to-client mapping belong to the owning module, not to the page.

## Smart container and props-only presentation

### `src/modules/counter/model/use-counter.ts`

```ts
import { useState } from "react";

export const useCounter = () => {
  const [count, setCount] = useState(0);
  const increment = () => setCount((current) => current + 1);

  return { count, increment };
};
```

One hook per `model/use-<name>.ts` file. Domain helpers and client models also live
in `model`; it contains no JSX. Effects need cleanup and requests need cancellation.
Do not add effects for values that can be derived during rendering.

### `src/modules/counter/containers/Counter.tsx`

```tsx
import { useCounter } from "../model/use-counter";
import { CounterView } from "../presentational/CounterView";

export const Counter = () => {
  const { count, increment } = useCounter();

  return <CounterView count={count} onIncrement={increment} />;
};
```

Containers call model hooks and supply computed view data and callbacks. Do not
define hooks or perform HTTP requests directly inside components.

### `src/modules/counter/presentational/CounterView.tsx`

```tsx
import { formatCount } from "@/common/libs/numbers/format-count";

interface CounterViewProps {
  readonly count: number;
  readonly onIncrement: () => void;
}

export const CounterView = ({ count, onIncrement }: CounterViewProps) => (
  <section aria-label="Counter">
    <output aria-label="Count">{formatCount(count)}</output>
    <button type="button" onClick={onIncrement}>
      Increment
    </button>
  </section>
);
```

Presentation renders props/slots and invokes supplied callbacks. It does not
import model/containers, call hooks, own domain state or perform browser/network
I/O. Reusable presentation belongs in `common/ui`, without a common barrel.

### `src/modules/counter/index.ts`

```ts
export { Counter } from "./containers/Counter";
```

Export only the supported consumer API. Other modules and pages never deep-import
`model`, `containers` or `presentational`.

## Shared utilities

### `src/common/libs/numbers/format-count.ts`

```ts
export const formatCount = (count: number): string => new Intl.NumberFormat().format(count);
```

Group framework-agnostic utilities under `common/libs/<lib>/`. Keep domain helpers
and mappers in the owning module's `model`; purity alone does not make them shared.
There is no mandatory `lib` directory inside a module.

## Contracts and dependency injection

- Wire types are generated from TypeSpec/OpenAPI into `web/.generated/api.d.ts`,
  outside `src`. Never recreate DTOs manually or edit generated types.
- A module may expose its own client model. Adapt wire data at its boundary so
  transport changes do not spread through presentation and unrelated modules.
- FEOD does not prescribe a transport module name. Keep the actual transport
  boundary from the task; do not create a second client or invent an HTTP module.
- Strong cross-module coupling uses an explicit interface and injection from app
  composition. Required dependencies are explicit; do not hide a missing dependency
  behind a console logger, a no-op or a fake implementation.
- `global` contains ambient declarations connected by TypeScript configuration,
  not imported application services.

## Verification

Run `npm run --workspace web check` from the repository root. It generates external
API types, type-checks, lints, scans layout and exercises plugin/architecture guards.
The examples do not replace actual runtime verification when a UI is implemented.
