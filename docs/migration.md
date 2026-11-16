# Legacy → новый Runlark

## Identity и сохранность

Старый репозиторий: `bondiano/runlark`, GitHub ID **1437343687**.
Целевое имя: **runlark-lagacy** — написание из запроса владельца сохранено.
Локальный путь legacy: `/Users/bondiano/workspace/runlark-lagacy`.
Новый репозиторий: отдельная Git history, private `bondiano/runlark`, локально прежний путь Runlark.

Старые branches, tags, PR, issues, releases, настройки, текущий checkout, ignored локальные данные
и ADR сохраняются в legacy. После перемещения main checkout выполнить `git worktree repair`
для сохранения зарегистрированных worktrees. Не reset/clean и не менять их branches.
Legacy продуктовая модель, migrations и Temporal runtime не импортируются в новую историю.

## До переключения

- Сохранить IDs, refs, ruleset и local status; подготовить и проверить новый bootstrap.
- Проверить свободное имя `runlark-lagacy` и отдельный scope infrastructure.
- Подготовить обеим identities независимые runner admission и Sych allowlist.
- Production rollout для trusted Sych config требует отдельного запроса владельца.

## Инфраструктура

В petinfra runner `githubConfigUrl` задан именем `https://github.com/bondiano/runlark`.
Нельзя считать, что новый repo с тем же URL получил старую runner registration.
Нужны отдельные ARC namespaces/pools для legacy и нового repo, без production identities.
Имена jobs сохраняются: `runlark-ci` и `runlark-review` в каждом repository scope.

Trusted Sych `scm.github[id=sych].repositories` должен содержать **оба** full names.
Существующий product writer client должен оставаться scoped к legacy; новая разработка
использует GitHub App review, а не автоматически наследует legacy API credentials.
CI selectors неизменны: `.github/workflows/ci.yml`, job `check`, artifact `sych-reports`,
`clippy.json`/`outcomes.json`, manifest `sych-ci.json`. Реальный bootstrap не публикует Clippy.

Sych App installation должна разрешать обе numeric repository identities.
Автоматическое включение нового private repository нельзя предполагать.
Не deploy другой reviewer revision/image и не менять models/budgets/policy.

## GitHub gates

Обнаруженный legacy ruleset `main` (ID 27033371) требует PR, linear history и запрещает
force push/deletion. В нём **нет** required status checks, вопреки старой documentation.
Его rename должен сохранить без ослабления. Новый ruleset должен явно требовать
current-branch `check` и `sych`, PR-only, linear history, no force push/deletion.

Первый seed commit нового repo необходим до существования base branch. Полный bootstrap
проходит отдельным PR; после seed защита включается до merge. Никакого force push истории.

## Приёмка переключения

- GitHub legacy имеет исходный numeric ID, новый Runlark — другой; оба private.
- Legacy current HEAD/branch/status и все зарегистрированные worktrees сохранены.
- Local remotes указывают на правильную identity; новый repo не наследует .git, .env или auth.
- Новый `mise exec -- just check` проходит; hooks настроены.
- Bootstrap PR проходит настоящий `check`, затем Sych current-head review.
- Findings, UNKNOWN и gaps разобраны; отсутствующие runners/App/evidence не объявляются зелёными.
- Codex sidebar показывает legacy по его новому пути и новый проект по прежнему.

Пока пункты внешней приёмки не наблюдались, переключение не считается завершённым.
