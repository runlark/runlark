# План поставки

## B0 — инициализация

- [x] Новый продуктовый план, архитектура, ограничения и визуальный контракт.
- [x] Переносимые development/review skills, pinned tooling, hooks, file-size/security/static gates.
- [x] Bootstrap запрещает продуктовые исходники без их compiler/architecture/test/smoke gates.
- [ ] GitHub cutover и проверка сохранённой legacy identity.
- [ ] Отдельные runners, Sych admission и required current-head `check`/`sych` для нового repo.
- [ ] Реальные CI и Sych bootstrap PR; findings/UNKNOWN/gaps разобраны.

## A0 — управляемая DSH-сессия и локальный durable run

Один Project, Task, Run, worktree и DSH session.

- [ ] Проверить и зафиксировать версию DSH, реальный Host bridge/controller extension point.
- [ ] Pure domain + SQLite: команда и назначение сохранены до отправки эффекта.
- [ ] Реальная сессия в правильном worktree доступна в DSH UI; события обновляют карточку.
- [ ] Send/Stop/resume/cwd/archive управляемой сессии проходят command layer Rust.
- [ ] Закрытие браузера не прерывает выполнение.
- [ ] Отдельный рестарт Rust и DSH не создаёт второй запуск; unknown сверяется.
- [ ] Вопрос сохраняется и после рестарта восстанавливается либо явно задаётся заново.
- [ ] Реальные gates Rust/SQLite/DSH/frontend, architecture evidence и failure-path smoke.

Выход: работающий прототип самой рискованной границы, а не имитация агента.

## A1 — solo alpha

- [ ] Полная доска и чат-команды; быстрый fix и обычная задача.
- [ ] Две параллельные задачи с отдельными worktrees; очередь, ожидание человека.
- [ ] Уточнение → план → реализация → проверки → человеческая приёмка.
- [ ] Ограниченные repair loops; attempts, time и spend limits, явная unknown cost.
- [ ] История, recovery, полноценный CLI и настоящее создание PR.
- [ ] WIT + package manifest + first-party workflow + настоящий WASM validator, digest pinning.
- [ ] Дизайн legacy в DSH, desktop/mobile, accessibility и i18n.
- [ ] Неделя реальных изменений Runlark через Runlark без ручного восстановления состояния.

## A2 — команда из 2–4 человек

- [ ] Один backend/protocol для SaaS и self-host, workspace roles и приглашения.
- [ ] Runner assignment без автоматического takeover активного run.
- [ ] Comments, decisions, durable history, verified artifacts и mobile acceptance.
- [ ] Snapshot/cursor/revision conflicts, offline drafts и reconnect failure paths.
- [ ] Solo→team только без активных runs, с явным server acknowledgement и ownership change.

Auto-merge, marketplace и полный remote session proxy не входят в первый полезный релиз.
