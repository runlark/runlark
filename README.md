# Runlark

[Репозиторий](https://github.com/runlark/runlark)

Локальная рабочая доска для разработки с агентами: поставь задачу, следи за выполнением,
ответь на вопрос и прими проверенное изменение в одном месте.

Новый продукт для одного человека и команд из 2–4 участников: интерфейс и агентные сессии
на базе DSH, Rust CLI/daemon для выполнения, SQLite для локального состояния, WASM для
расширений процесса. Совместная работа подключается отдельно: SaaS или тот же self-host сервер.

## Состояние

Реализованы этапы **B0, A0, A1** (кроме недели dogfooding) и **A2**:

- Rust workspace: `crates/runlark-domain`, `crates/runlark-local` (SQLite + CAS + reconciliation),
  `crates/runlark-wasm` (WIT/Wasmtime host + `workflows/solo-default`), `crates/runlark-cli`
  (CLI + локальный daemon, 2-слотовый scheduler, worktrees, repair loop, PR, team sync/runner),
  `crates/runlark-server` (Axum + PostgreSQL сервер совместной работы для SaaS и self-host).
- DSH интеграция и UI: `packages/dsh-plugin` (Host bridge, `SessionCommandInterceptor` патч для
  `dsh 0.2.2-rc.3`, proxy-маршруты и slash-инструмент) и `web/` (канбан-доска, экран задачи,
  командная работа, CAS-проверка и mobile acceptance по `web/DESIGN.md`).
- Старый Runlark сохраняется отдельно как `runlark-lagacy`, с исходной историей, ветками и ADR.
- Отложено за первый релиз: container-профиль изоляции, формальная модель state machine,
  полный remote DSH session proxy, auto-merge и marketplace.

- [Продукт и принятые ограничения](PRODUCT.md)
- [Архитектура и границы](docs/architecture.md)
- [Запуск и упаковка (solo и self-host)](docs/running.md)
- [План поставки и критерии](ROADMAP.md)
- [Разработка](docs/development.md) и [гейты](docs/gates.md)
- [DSH spike и контракт перехватчика](docs/dsh-spike.md)
- [Исходный новый план](docs/new-plan.md)
- [Визуальный контракт](web/DESIGN.md)
- [Переключение legacy и внешних интеграций](docs/migration.md)

## Разработка и сборка

```sh
scripts/setup
mise exec -- just check
mise exec -- just package
```

`just check` запускает полный локальный и CI gate: статические проверки, размер файлов,
архитектуру (`easement check --require-evidence`), Rust (`fmt`, `clippy -D warnings`, `machete`,
`cargo deny`, `rustdoc`), WASM/WIT, TypeScript/FEOD/Oxlint/UI, тесты (JS/TS, SQLite, PostgreSQL,
упаковка release-архива) и сквозной E2E smoke (A0 + A1 + A2).
Инструкции по запуску solo и self-host `runlark-server` — в [docs/running.md](docs/running.md).

Новый репозиторий публичный; CI и ожидание Sych работают на GitHub-hosted `ubuntu-24.04`.
Проект распространяется под лицензией [Apache License 2.0](LICENSE).
Сторонние навыки сохраняют свои лицензии и сведения о происхождении.
