# Гейты разработки

Разработка нового Runlark сохраняет строгую проверку независимо от того, что Sych в самом
продукте станет необязательным reviewer.

## Исполняется сейчас

`mise exec -- just check` — общий локальный, pre-push и CI gate:

- `lint-scope`: продуктовые исходники запрещены до интеграции их настоящих гейтов.
- `lint-size`: существующий проверенный индексный лимит 20 000 байт; legacy baseline не перенесён.
- `lint-files`: Oxfmt, Markdown, TOML, EditorConfig, shell, actions, workflow security, typos,
  just formatting и offline local links/fragments.
- `lint-skills`: синтаксис сохранённых launchers и browser scripts.
- `security`: npm audit (low и выше), redacted Gitleaks.
- `test`: настоящие behavioral regressions file-limit и source-admission на временных Git repo.

Pre-commit запускает `just lint`; commit-msg требует Conventional Commits.
Pre-push очищает exported Git environment, проверяет сообщения всех новых отправляемых коммитов
по каждому ref и запускает полный `just check`. Существующая опубликованная история исключается
по remote-tracking refs и прежним remote OID; перед новой веткой требуется fetch.
`scripts/setup` включает tracked hooks и ставит pinned dependencies.
Допустимые типы и примеры — в [разработке](development.md#conventional-commits).
Изменение политик не заменяет исправление обнаруженного дефекта.

## Вместе с первой реализацией

Scope guard снимается для языка только тем же PR, который подключает соответствующие проверки:

| Область       | Требуемые проверки                                                                                                                                                                                                                                             |
| ------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Rust          | pinned stable toolchain; workspace lints (forbid unsafe, deny unused must-use, Clippy all/pedantic with -D warnings); rustfmt; cargo-machete; sych-style; cargo deny; committed lockfile; all-target/all-feature build, tests, rustdoc; реальный process smoke |
| Architecture  | owner-declared `easement.yml`, pure domain, no I/O in transitions/transactions, require-evidence indexers, policy examples, no baseline debt                                                                                                                   |
| DSH/React     | pinned verified DSH; TypeScript strict; Oxlint React/hooks/accessibility; FEOD public APIs and layout/architecture tests; Reatom/Form rules; desktop/mobile rendered scenarios; Impeccable detect                                                              |
| Contracts     | generated types и drift check для выбранного versioned command/event contract; WIT compatibility/package validator tests                                                                                                                                       |
| Storage       | все новые migrations linted; transaction/restart/dedup/conflict tests на настоящей SQLite; real PostgreSQL tests при появлении server                                                                                                                          |
| Containers    | digest pins, Hadolint, реальные limits/egress/credentials/isolation scenarios для отдельного container-профиля                                                                                                                                                 |
| State machine | observable transition/failure tests; legacy Lean model не считается доказательством новой модели, перенос proof/equivalence gates после адаптации                                                                                                              |

`rust-toolchain.toml`, `rustfmt.toml` и `deny.toml` сохраняют старые стандарты, но отсутствие
Cargo workspace не выдаётся за Rust validation. Legacy API, Temporal histories, migrations
0001–0024 exceptions, container smoke и Lean model относятся к старой реализации.

## GitHub и Sych

`CI / check` и `Sych / sych` работают на стандартных GitHub-hosted `ubuntu-24.04`.
Новый `runlark/runlark` публичный; CI не требует личных self-hosted runners или ARC registration.
Actions pinned по SHA; read-only permissions; checkout не сохраняет credentials;
`pull_request_target` не используется. CI и ожидание Sych — отдельные workflows.

`scripts/ci-check` сохраняет реальный exit code и tested checkout SHA, включая merge checkout,
в `sych-reports` (`outcomes.json`, `sych-ci.json`). Bootstrap не создаёт фиктивный `clippy.json`.
При появлении Rust Clippy должен публиковать настоящую Cargo JSON диагностику и CI manifest
должен указывать настоящий compiler/target/features. Missing evidence остаётся missing.

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
