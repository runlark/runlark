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

## Conventional Commits

Новые коммиты обязательно следуют [Conventional Commits 1.0.0](https://www.conventionalcommits.org/en/v1.0.0/):
`type(scope)!: description`. Scope и `!` необязательны; описание непустое.
Типы проекта: `feat`, `fix`, `docs`, `style`, `refactor`, `perf`, `test`, `build`, `ci`, `chore`, `revert`.
Регистр type не ограничен; для единообразия используем lowercase. Тело и footer начинаются после
пустой строки. Breaking change отмечается `!` или footer `BREAKING CHANGE: ...`.

Примеры: `feat(board): add task creation`, `fix(local): reconcile interrupted run`,
`chore(git): enforce Conventional Commits`, `feat(api)!: require command revision`.
Revert оформляется как `revert: ...`; незавершённые `fixup!`/`squash!` не отправляются.
Заголовок PR тоже должен следовать формату, чтобы squash merge сохранил его.

`scripts/setup` включает `.githooks`: `commit-msg` проверяет сообщение до создания коммита;
`pre-push` повторно проверяет все новые коммиты каждого отправляемого ref, затем запускает полный
`just check`. Уже опубликованная история не переписывается. Перед отправкой новой ветки выполните
`git fetch origin`: remote-tracking refs определяют уже опубликованную историю. Если Git не может
прочитать исходный commit, проверка завершается ошибкой, а не пропускает его. Для push по URL без
configured remote проверяется вся достижимая история.

Проверка формата не определяет семантическую корректность выбранного типа и не заменяет review.
Как и другие локальные hooks, она требует установленного `core.hooksPath`; отключать её нельзя.

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

Новый репозиторий публичный; все CI jobs, включая ожидание Sych, работают на стандартных
GitHub-hosted `ubuntu-24.04`. Личные self-hosted runners и private infrastructure для CI не нужны.
Проект распространяется под лицензией [Apache License 2.0](../LICENSE); лицензии сторонних навыков сохраняются.
Не коммитить credentials, личные transcripts, production data или local
checkout paths. Не deploy production, не создавать платные ресурсы и не публиковать images без запроса.
