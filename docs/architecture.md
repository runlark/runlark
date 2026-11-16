# Архитектура нового Runlark

Это целевая архитектура из [нового плана](new-plan.md), а не перечень реализованных компонентов.
Bootstrap содержит документацию и инструменты проверки; runtime начинается с этапа A0.

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

## Планируемые границы исходников

Границы создаются вместе с первой реализацией, без пустых crates и фиктивных адаптеров.

```text
crates/runlark-domain/    pure types, commands, transitions
crates/runlark-local/     SQLite writer, journal/outbox, recovery
crates/runlark-cli/       CLI and local daemon composition
crates/runlark-wasm/      versioned WIT host and package validation
crates/runlark-server/    optional collaboration, after solo alpha
packages/dsh-plugin/     host bridge, tools, managed DSH profile
web/                    reusable React UI and FEOD modules mounted through DSH
wit/                    versioned plugin contracts
```

Domain не зависит от storage/runtime/I/O. Local writer атомарно сохраняет переход, журнал и outbox;
внешние effects выполняются после commit. CLI, UI и chat tools используют одну command layer.
Авторизация проверяется на границе и перед внешним действием. Decisions и publish ожидают
конкретную ревизию результата. Unknown delivery не повторяется с новой identity.

До первого файла или импорта объявить реальные компоненты в `easement.yml`, выполнить
`easement explain`/`propose` и включить `easement check --require-evidence` в `just check`.
Scope guard не разрешает добавлять продуктовый код до подключения этих гейтов.

## DSH spike

Зафиксировать commit DSH и supported API. Проверить managed session admission, cwd/permissions,
UI mounting, события, остановку и отдельные перезапуски. При необходимости controller patch
живет в одном адаптере с контрактными тестами. SDK worker не подменяет проверку Host bridge.

Frontend сохраняет React/Mantine adapters, Reatom v1001/Form, Paraglide и FEOD. DSH composition
требует проверки совместимости; это не разрешение менять router или вводить `@reatom/jsx`.
Перед UI-работой: Impeccable context/playbook/craft floor и modern-web-guidance; после —
живые desktop/mobile сценарии, detect и настоящие compiler/lint/architecture/interaction gates.

## Сохранение дизайна

[web/DESIGN.md](../web/DESIGN.md) скопирован из legacy без изменения.
Legacy UI kit и brand assets остаются в исходном репозитории до переноса настоящего интерфейса.
Не переносить старый API/Work/Temporal runtime как скрытую зависимость новой доски.
