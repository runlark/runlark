# Гейты разработки

Разработка нового Runlark сохраняет строгую проверку независимо от того, что Sych в самом
продукте станет необязательным reviewer.

## Исполняется сейчас

`mise exec -- just check` — общий локальный, pre-push и CI gate (`lint`, `test`, `docs`, `smoke`):

- `lint-scope`: `scripts/bootstrap-scope.mjs` пропускает только языки с подключёнными гейтами
  (Rust, TypeScript/TSX, CSS, SQL, WIT, workflow WASM/WAT, `scripts/*.mjs`) и запрещает
  неинтегрированные форматы (`Dockerfile`, `.lean`, `.py`, `.go`, `.tsp` и др.).
- `lint-size`: индексный лимит 20 000 байт на рукописный файл; legacy debt baseline отсутствует.
- `lint-arch`: `easement check --require-evidence` по [easement.yml](../easement.yml) (чистый
  domain, изоляция `runlark-server` от SQLite, границы `runlark-wasm`, `dsh-plugin` и FEOD).
- `lint-rust`: pinned stable Rust `1.95.0`, `cargo fmt --all --check`,
  `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo machete`,
  `cargo deny check` (advisories, licenses, bans, sources).
- `lint-wasm`: `cargo test -p runlark-wasm` (WIT-контракт `runlark:workflow@0.1.0`, проверка
  manifest/SHA-256 и ограниченное исполнение `workflows/solo-default` в Wasmtime).
- `lint-ts` и `lint-ui`: strict TypeScript (`packages/dsh-plugin` host и UI), Oxlint
  (React/hooks/a11y), FEOD-структура (`feod.profile.md`) и UI-границы (`scripts/lint-ui.mjs`).
- `lint-files` и `lint-skills`: Oxfmt, Markdown, TOML, EditorConfig, shell (`shfmt`, `shellcheck`),
  actions (`actionlint`, `zizmor --pedantic`), typos, `just --fmt --check`, offline локальные
  ссылки/якоря (`lychee`) и синтаксис skill launchers.
- `security`: `npm audit` (low и выше), redacted Gitleaks.
- `test`: behavioral tests (`scripts/*.test.mjs` — scope, file-limit, commit hooks, UI-карточка/
  доска/команда, сборка и проверка release-архива), тесты `packages/dsh-plugin/test/*.test.ts`,
  workspace-тесты Rust (включая SQLite миграции, outbox, recovery) и реальные PostgreSQL-тесты
  `runlark-server` через `scripts/with-test-postgres.mjs`.
- `docs`: `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features`.
- `smoke`: сквозной многопроцессный E2E smoke (`scripts/smoke.mjs`: A0 + A1 + A2 с реальным
  SQLite, PostgreSQL `runlark-server`, 2 раннерами, DSH host bridge и собранным UI). Дополнительно
  локально доступен `scripts/smoke-dsh-e2e.mjs` против установленного бинарника `dsh`.

Pre-commit запускает `just lint`; commit-msg требует Conventional Commits.
Pre-push очищает exported Git environment, проверяет сообщения всех новых отправляемых коммитов
по каждому ref и запускает полный `just check`. Существующая опубликованная история исключается
по remote-tracking refs и прежним remote OID; перед новой веткой требуется fetch.
`scripts/setup` включает tracked hooks и ставит pinned dependencies.
Допустимые типы и примеры — в [разработке](development.md#conventional-commits).
Изменение политик не заменяет исправление обнаруженного дефекта.

## Гейты для будущих расширений

Rust, Architecture (`easement.yml`), DSH/React/FEOD, WIT/WASM и SQLite/PostgreSQL уже подключены в
`just check`. Scope guard по-прежнему закрыт для областей, которые ещё не реализованы; снятие
запрета допускается только тем же PR, который подключает соответствующие проверки:

| Область       | Требуемые проверки                                                                                                                                                                                      |
| ------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Containers    | digest pins, Hadolint, реальные limits/egress/credentials/isolation scenarios для отдельного container-профиля (пока `Dockerfile` запрещён в `scripts/bootstrap-scope.mjs`)                             |
| State machine | observable transition/failure tests уже работают; legacy Lean model не считается доказательством новой модели, перенос proof/equivalence gates требует интеграции проверок вместе с `.lean` исходниками |

Legacy API, Temporal histories, migrations 0001–0024 exceptions, container smoke и Lean model
относятся к старой реализации (`runlark-lagacy`).

## GitHub и Sych

`CI / check` и `Sych / sych` работают на стандартных GitHub-hosted `ubuntu-24.04` (с сервисным
контейнером `postgres:16-alpine` для реальных тестов `runlark-server`).
Новый `runlark/runlark` публичный; CI не требует личных self-hosted runners или ARC registration.
Actions pinned по SHA; read-only permissions; checkout не сохраняет credentials;
`pull_request_target` не используется. CI и ожидание Sych — отдельные workflows.

`scripts/ci-check` сохраняет реальный exit code `just check`, настоящую JSON-диагностику
`cargo clippy --workspace --all-targets --all-features --message-format=json` (`clippy.json`) и
метаданные проверенного checkout SHA, включая merge checkout, в `sych-reports` (`outcomes.json`,
`sych-ci.json`, `clippy.json`). Missing evidence остаётся missing.

Существующий upload-artifact `continue-on-error` сохранён: ошибка storage quota видна Sych
как missing evidence и не подменяет exit code основного gate.

`main` требует PR, актуальную ветку, `check` и `sych`; force push и deletion запрещены.
После каждого push — дождаться CI и `scripts/wait-sych` на точном current head, прочитать
все findings, UNKNOWN и coverage gaps. Наличие summary не означает approval.
Ожидаемый автор summary — `runlark-sych-reviewer[bot]`, отдельный приватный GitHub App
организации `runlark`. Чужой bot или summary старого head не принимается.
Нельзя менять reviewer image/models/budgets ради зелёного результата.

Файлы workflow не доказывают Sych admission. Реальные проверки App installation, server
allowlist, hosted execution и GitHub ruleset перечислены в [migration](migration.md).
