# Новый план Runlark

Источник: [Новая архитектура Runlark, общий чат](https://chatgpt.com/share/6afabca7-06c0-83eb-8ad1-24e65ad9c4ec).
Прочитан 16 ноября 2026. Ниже — требования для нового репозитория; исследовательские утверждения
о DSH и сторонних проектах из чата нужно заново проверить на выбранных версиях перед реализацией.

## Направление

«T3 Code на канбане», встроенный в DSH: agentic workflow для маленькой команды с полноценным
solo-режимом, Rust CLI/daemon, WASM-расширениями и необязательной collaboration.
Референсы: PurpleFlow, Aixle Flow, Codync и T3 Code; это ориентиры сценариев, не зависимости.

## Состав

1. DSH frontend plugin: доска, задача, чат, diff, приёмка и inbox решений.
2. DSH Host bridge: команды Rust ↔ DSH-сессии, события, вопросы и управление сессиями.
3. Rust daemon: задачи, runs/steps/attempts, очередь, worktrees, budgets, decisions, recovery.
4. SQLite: состояние, журнал, команды/ответы, outbox и sync cursors. Крупные артефакты — CAS.
5. WASM host: Component Model/WIT, ограниченные capabilities и версионируемые пакеты.
6. Optional Rust/Axum + PostgreSQL server: общая доска, роли, runners, история и артефакты.

Один пакет DSH может включать модули UI, host, tools, agent profiles и theme.
Rust не вызывает модели: DSH делает agent loop. Долгие ожидания принадлежат host и БД,
а не живому Promise или WASM instance.

## Критический технический риск

Send, Stop, resume, cwd и archive управляемой сессии обязаны проходить через Rust:
действие → сохранённая команда → операция DSH. Штатный DSH controller может обращаться к
агенту напрямую. Spike проверяет extension point; если его нет, допускается небольшой явно
изолированный upstream patch. До проверки нельзя обещать интеграцию одним плагином.

## Семантика

Project, Task, Run, Step/Attempt, AgentSession, Decision, Artifact/Evidence и Runner — разные
сущности. Run фиксирует ревизию задачи, процесс, plugin digest/config и Git base.
Команда содержит stable ID, expected revision, неизменяемый payload и проверенного автора.
Повтор возвращает прежний результат; изменённое тело требует нового ID.
Принятое намерение и завершённая операция — разные факты.

Закрытие UI не останавливает работу. Рестарт daemon начинает с reconciliation. Падение DSH
означает interrupted/unknown до сверки истории и effects. Stop сначала cancel requested,
затем подтверждённая остановка. Решения сохраняются независимо от DSH Promise.

Первый лимит — две выполняющиеся задачи; ожидание человека освобождает слот.
Каждый одновременно пишущий run имеет отдельный worktree. Управляемые publish/merge
проверяют текущие права и версии; native mode с локальными credentials не обещает container isolation.

## WASM alpha

Минимальный WIT, проверяемый manifest, один first-party workflow, один настоящий WASM validator,
pinning пакета на весь run. Старый digest остаётся доступен незавершённым runs.
Node/TS host plugin доверенный; ограничения WASM автоматически не ограничивают host или UI bundle.

## Collaboration

Одинаковый протокол для SaaS и self-host. Сервер упорядочивает общие изменения и назначает
ровно одного runner. При потере связи нет автоматического takeover: локальные проверки могут
завершиться, общие start/accept/publish ждут сервера. Stop остаётся запросом до подтверждения.

Команды дедуплицируются по ID; изменения Task проверяют ревизию; комментарии append-only;
run events имеют owner stream и последовательность; клиент читает snapshot/cursor и события;
артефакты проверяются по hash. Применение изменений и cursor атомарно. Черновики сохраняются.
Задачи, обсуждения, планы, решения, durable transcripts, diff и проверки синхронизируются;
credentials и локальные checkout paths не становятся общими настройками.

Перевод solo-проекта в team сначала допускается только без активных runs: snapshot, server ack,
явное изменение ownership. Полный удалённый DSH session proxy отложен; сначала сохранённая
история и очередь команд. Сервер не обязателен для solo.

## Последовательность

Сначала доказать управляемую DSH-сессию с durable local run; затем solo alpha и неделя
реальных собственных задач без ручного восстановления; потом collaboration для 2–4 человек.
Критерии каждого этапа — в [ROADMAP](../ROADMAP.md).

Legacy ADR не переписываются. Решения о новом направлении фиксируются в этом новом репозитории;
новые ADR создаются только по прямому запросу владельца.
