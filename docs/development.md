# Разработка

```sh
scripts/setup
mise exec -- just check
```

Нужны Git, mise и Bash; `scripts/setup` устанавливает только pinned project tools, npm dependencies
по lockfile и tracked hooks. Перед новым языком или runtime подключить [соответствующие гейты](gates.md).

Обычный flow: ветка `codex/...` от свежего `origin/main` → smallest complete change → реальное
поведение и failure paths → `mise exec -- just check` → PR → green CI и current-head Sych.
Не reset/clean чужие изменения. Не bypass hooks, checks, timeout или review gaps.

Сначала читать [PRODUCT](../PRODUCT.md), [архитектуру](architecture.md), [ROADMAP](../ROADMAP.md)
и применимый `.agents/skills/*/SKILL.md`. Имплементации сейчас нет; команды продукта из нового
плана пока не существуют. Дизайн-контракт сохранён в [web/DESIGN.md](../web/DESIGN.md).

## Полезные команды

| Команда               | Действие                                        |
| --------------------- | ----------------------------------------------- |
| `just fmt`            | форматирование поддерживаемых authored файлов   |
| `just lint`           | static/security/scope/size gates                |
| `just test`           | behavioral tests инструментов bootstrap         |
| `just check`          | весь проверяемый bootstrap                      |
| `just review PR`      | дождаться настоящего Sych summary текущего head |
| `just impeccable ...` | сохранённый project-local UI workflow           |

[Исходный план](new-plan.md) содержит исследовательские утверждения о внешних API. Перед SDK,
DSH bridge/controller, Wasmtime/WIT или frontend implementation сверить официальные docs и
конкретную версию. Не добавлять зависимости и абстракции для будущих, ещё не используемых компонентов.

Репозиторий приватный. Не коммитить credentials, личные transcripts, production data или local
checkout paths. Не deploy production, не создавать платные ресурсы и не публиковать images без запроса.
