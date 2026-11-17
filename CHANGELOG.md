# Changelog

## Unreleased

- Реализован фундамент и сквозной вертикальный срез A1 (solo alpha):
  - Расширен `runlark-domain` и `runlark-local` (миграция `0004_a1_board_and_history.sql`, `Project`, `Task`, `Attempt`, `Decision`, `BudgetPolicy`, `CostSummary`, `BoardCommand`, CAS-хранилище артефактов `ArtifactCasStore`, проекция доски `BoardProjection`, FIFO-очередь и восстановление после рестарта).
  - Добавлен WASM host `crates/runlark-wasm`, контракт `wit/runlark-workflow.wit` и first-party workflow-пакет `workflows/solo-default` (manifest, declarative stages, WASM validator с digest pinning, ограничениями fuel/memory и изоляцией capabilities).
  - Реализованы 2-слотовый `Scheduler` с отдельными git worktrees на каждый активный run, ограниченный repair loop с бюджетами и явной `UnknownCost`, привязанная человеческая приёмка и создание PR (`git push` + `gh pr create`), единый парсер/диспетчер чат-команд `/task`, `/fix`, `/plan`, `/run`, `/stop`, `/resume`, `/answer`, `/accept`, `/reject`, `/pr`, `/status`, `/board`, `/budget`, полноценный CLI (`project`, `task`, `run`, `question`, `chat`, `plugin`) и HTTP API `/api/v1/*`.
  - В `packages/dsh-plugin` добавлены proxy-маршруты `/api/runlark/{board,projects,tasks,chat,runs,plugins}`, инструмент `runlark_chat_command`, `UsageBridge` (явно сохраняющий отсутствующую стоимость как `null` `UnknownCost`) и регистрация вкладки `Runlark Board` в DSH right-sidebar/dockkit.
  - В `web/` реализована полная канбан-доска (5 колонок, drag-and-drop + доступное меню Move, фильтры, диалог создания задачи) и экран задачи (уточнение, план, попытки/бюджет, diff/артефакты CAS, приёмка/PR, живые события и slash-чат) по контракту `web/DESIGN.md` с desktop/mobile адаптацией, keyboard accessibility и RU/EN i18n.
  - Добавлены UI-тесты (`scripts/ui-board.test.mjs`) и сквозной A1 smoke (`scripts/smoke-a1.mjs`), интегрированный в `scripts/smoke.mjs`.
- Завершён этап A0: управляемая DSH-сессия и локальный durable run готовы к переходу к A1 (solo alpha).
- Реализован durable command layer и outbox-диспетчер (`crates/runlark-cli`, `crates/runlark-local`): атомарное сохранение намерения в SQLite до внешнего эффекта (`pending -> dispatched -> acked | failed`), ретраи со сбросом `error_message`, сохранение `unknown` (504/таймаут) до сверки, идемпотентность и 409 при конфликте payload или `expected_revision`.
- Реализовано управление изолированными git worktrees (`$WORKSPACE/.runlark/worktrees/<run_id>`) и CWD confinement для запускаемых сессий.
- Подготовлен патч перехватчика команд для `@deepseek-ai/dsh-api-session-controller@0.2.2-rc.3` (`SessionCommandInterceptor`), host bridge с динамической синхронизацией ревизий, HTTP-маршруты `/api/runlark/*`, `DshSubscriber` для подписки на `session/follow` (`/api/remote.mux`) и сверка `turn/start` / `turn/end` (включая crash-repair `interrupted` и `CancelRequested -> Stopped`).
- Реализована сквозная обработка человеческих вопросов `WaitingForApproval` (`ask_user_question`): сохранение в SQLite, выживание после раздельных рестартов Rust-демона и DSH, автооткрытие вкладки и восстановление карточки вопроса в DSH UI, а также продолжение run после ответа.
- Построен и подключён frontend DSH UI-карточки по правилам FEOD, Mantine и Impeccable (`web/src`, `packages/dsh-plugin/src/ui`), а также расширен A0 failure-path smoke (`scripts/smoke.mjs` и `scripts/smoke-dsh-e2e.mjs` с реальным `dsh 0.2.2-rc.3`).
- Зафиксирован DSH spike (`deepseek-ai/deepseek-harness`, `dsh-v0.2.2-rc.3`) и спецификация перехватчика команд (`docs/dsh-spike.md`).
- Подключены архитектурные границы `easement.yml` (`--require-evidence`), реальные Rust/TS/SQLite/process-smoke гейты и вертикальный срез A0 (`runlark-domain`, `runlark-local`, `runlark-cli`, `packages/dsh-plugin`).
- Для организации runlark настроен отдельный приватный Sych App; current-head waiter ожидает `runlark-sych-reviewer[bot]`.
- Новый репозиторий публичный; CI и ожидание Sych перенесены на GitHub-hosted `ubuntu-24.04`.
- Git hook tests используют project-local mise pins и пустой global config на чистых hosted runners.
- Conventional Commits обязательны: commit-msg проверяет сообщение, pre-push проверяет все новые коммиты перед полным `just check`.
- Целевой GitHub-репозиторий нового проекта: `runlark/runlark`, отдельно от legacy `bondiano/runlark`.
- Новый local-first Runlark: DSH UI/runtime + Rust CLI/daemon + SQLite + WASM; optional collaboration.
- Сохранены исходный план, визуальный контракт и development/review guidance legacy.
- Инициализированы реальные bootstrap checks, security, tracked hooks и current-head Sych workflow.
- Продуктовые исходники требуют одновременного подключения compiler/architecture/test/smoke gates.
