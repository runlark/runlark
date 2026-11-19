# Архитектура нового Runlark

Документ описывает реализованную архитектуру этапов A0–A2 на основе [нового плана](new-plan.md)
и зафиксированные границы компонентов в [easement.yml](../easement.yml). Отдельный
container-профиль изоляции, формальная модель state machine, полный remote DSH session proxy,
auto-merge и marketplace остаются вне текущей реализации.

## Владение

| Компонент       | Владеет                                                                     | Не принимает на себя                 |
| --------------- | --------------------------------------------------------------------------- | ------------------------------------ |
| DSH UI          | отображение доски, чата, результатов, пользовательское UI-состояние         | durable workflow state               |
| DSH Host bridge | адаптация команд и событий, связь с AgentSession                            | самостоятельные решения о run        |
| DSH runtime     | разговор, модель, контекст, tools, agent loop                               | приёмка задачи                       |
| Rust daemon     | Task/Run/Step/Attempt, команды, очередь, решения, worktrees, восстановление | вызовы моделей напрямую              |
| SQLite + CAS    | локальные записи и журнал, outbox, артефакты                                | неявная общая командная истина       |
| WASM host       | capabilities, bounded execution, версии пакетов                             | доверие произвольному Node host коду |
| Optional server | общая доска, права, порядок команд, назначения runner, история              | исполнение кода по умолчанию         |

## Границы исходников

Границы компонентов зафиксированы в [easement.yml](../easement.yml) и проверяются в `just check`:

```text
crates/runlark-domain/    pure types, commands, transitions, team sync contracts
crates/runlark-local/     SQLite writer, journal/outbox, CAS, team sync store, recovery
crates/runlark-wasm/      versioned WIT host, digest pinning, bounded Wasmtime validation
crates/runlark-cli/       CLI and local daemon composition (scheduler, worktrees, sync/runner)
crates/runlark-server/    optional SaaS/self-host collaboration server over PostgreSQL + CAS
packages/dsh-plugin/      host bridge, SessionCommandInterceptor patch, web proxy, DSH UI slots
web/                      reusable React UI (FEOD, Reatom v1001, Mantine adapters in common/ui)
wit/                      versioned WIT contracts (runlark:workflow@0.1.0)
workflows/solo-default/   first-party workflow manifest, declarative stages and WASM validator
```

Domain не зависит от storage/runtime/I/O. Local writer атомарно сохраняет переход, журнал и outbox;
внешние effects выполняются после commit. `runlark-server` изолирован от `runlark-local`,
`runlark-cli` и `rusqlite`, опираясь только на `runlark-domain` и PostgreSQL. CLI, UI и chat tools
используют одну command layer. Авторизация проверяется на границе и перед внешним действием.
Decisions и publish ожидают конкретную ревизию результата. Unknown delivery не повторяется с новой
identity.

`easement check --require-evidence` входит в обязательный `just check`. Перед добавлением файла или
межкомпонентного импорта проверяйте `easement explain`/`propose`. Scope guard
(`scripts/bootstrap-scope.mjs`) по-прежнему закрыт по умолчанию для ещё не подключённых форматов
исходников (например, `Dockerfile`, `.lean`, `.py`, `.go`).

## Интеграция DSH

Результаты spike зафиксированы в [dsh-spike.md](dsh-spike.md) на ревизии
`deepseek-ai/deepseek-harness` `dsh-v0.2.2-rc.3`. Патч перехватчика команд
`SessionCommandInterceptor` изолирован в `packages/dsh-plugin/patches/` и проверяется/применяется
через `scripts/apply-dsh-patch.mjs`.

Frontend использует React/Mantine adapters (Mantine разрешён только в `web/src/common/ui/**`),
Reatom v1001 (`withAsyncData`/`withAsync`) и FEOD ([feod.profile.md](../feod.profile.md));
`@reatom/jsx` запрещён архитектурной политикой. Перед UI-изменениями применяются Impeccable и
modern-web-guidance; после — живые desktop/mobile сценарии, detect и `just check`.

## Сохранение дизайна

[web/DESIGN.md](../web/DESIGN.md) — активный визуальный контракт, реализованный в `web/` и
упакованный в клиентский бандл `@runlark/dsh-plugin`. Старый API/Work/Temporal runtime из legacy не
импортируется.
