# План поставки

## B0 — инициализация

- [x] Новый продуктовый план, архитектура, ограничения и визуальный контракт.
- [x] Переносимые development/review skills, pinned tooling, hooks, file-size/security/static gates.
- [x] Bootstrap запрещает продуктовые исходники без их compiler/architecture/test/smoke gates.
- [x] Локальный legacy переименован, его refs/HEAD/status сохранены, все 15 worktrees восстановлены.
- [x] Владелец выбрал отдельную remote identity `runlark/runlark`; legacy URL не переиспользуется.
- [x] GitHub publication и проверка сохранённой legacy identity.
- [x] Все CI jobs используют стандартные GitHub-hosted `ubuntu-24.04`, без личных runners.
- [x] Sych admission и required current-head `check`/`sych` для нового public repo.
- [x] Реальные CI и Sych bootstrap PR; findings/UNKNOWN/gaps разобраны.

## A0 — управляемая DSH-сессия и локальный durable run

Один Project, Task, Run, worktree и DSH session.

- [x] Проверить и зафиксировать версию DSH, реальный Host bridge/controller extension point.
- [x] Pure domain + SQLite: команда и назначение сохранены до отправки эффекта.
- [x] Реальная сессия в правильном worktree доступна в DSH UI; события обновляют карточку.
- [x] Send/Stop/resume/cwd/archive управляемой сессии проходят command layer Rust.
- [x] Закрытие браузера не прерывает выполнение.
- [x] Отдельный рестарт Rust и DSH не создаёт второй запуск; unknown сверяется.
- [x] Вопрос сохраняется и после рестарта восстанавливается либо явно задаётся заново.
- [x] Реальные gates Rust/SQLite/DSH/frontend, architecture evidence и failure-path smoke.

Выход: A0 завершён — работающий прототип самой рискованной границы (реальный DSH + Rust/SQLite + UI), готовность к A1 (solo alpha).

## A1 — solo alpha

- [x] Полная доска и чат-команды; быстрый fix и обычная задача.
- [x] Две параллельные задачи с отдельными worktrees; очередь, ожидание человека.
- [x] Уточнение → план → реализация → проверки → человеческая приёмка.
- [x] Ограниченные repair loops; attempts, time и spend limits, явная unknown cost.
- [x] История, recovery, полноценный CLI и настоящее создание PR.
- [x] WIT + package manifest + first-party workflow + настоящий WASM validator, digest pinning.
- [x] Дизайн legacy в DSH, desktop/mobile, accessibility и i18n.
- [ ] Неделя реальных изменений Runlark через Runlark без ручного восстановления состояния.

## A2 — команда из 2–4 человек

- [x] Один backend/protocol для SaaS и self-host, workspace roles и приглашения.
- [x] Runner assignment без автоматического takeover активного run.
- [x] Comments, decisions, durable history, verified artifacts и mobile acceptance.
- [x] Snapshot/cursor/revision conflicts, offline drafts и reconnect failure paths.
- [x] Solo→team только без активных runs, с явным server acknowledgement и ownership change.

Auto-merge, marketplace и полный remote session proxy не входят в первый полезный релиз.
