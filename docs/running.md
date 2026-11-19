# Запуск и упаковка Runlark

Руководство покрывает три сценария:

1. Сборка и проверка автономного release-архива (`.tar.gz` + `.sha256`).
2. Локальный запуск **solo-режима** (`runlark daemon` + DSH Web с профилем `runlark`).
3. Self-host запуск командного сервера **`runlark-server`** (PostgreSQL + CAS) для команды из 2–4 человек.

Все команды ниже соответствуют текущей реализации (`runlark` CLI/daemon, `runlark-server`,
`@runlark/dsh-plugin`, `workflows/solo-default`).

## Требования

- **Solo (из release-архива):**
  - macOS (`aarch64-apple-darwin`) или Linux (`x86_64-unknown-linux-gnu`);
  - Git (для изолированных worktrees);
  - Node.js 24 (`>=24 <25`), `pnpm` и `patch` (для установки и проверки DSH-профиля);
  - DeepSeek Harness (`dsh`), совместимый с зафиксированной версией `0.2.2-rc.3`
    (`@deepseek-ai/dsh@0.2.2-rc.3`, см. [dsh-spike.md](dsh-spike.md));
  - `gh` CLI (только для создания PR через `runlark task pr`).
- **Сборка из исходников:** дополнительно `mise` (ставит pinned Rust `1.95.0`, Node `24.22.1`,
  `just` и инструменты проверок через `scripts/setup`).
- **Self-host `runlark-server`:** PostgreSQL 16+ и постоянный каталог на диске для CAS-артефактов.

## 1. Сборка release-архива

Из корня репозитория (после `scripts/setup`):

```sh
mise exec -- just package
```

Скрипт [package-release.mjs](../scripts/package-release.mjs):

1. Собирает release-бинарники `runlark` и `runlark-server` (`cargo build --release`).
2. Собирает браузерный бандл DSH UI (`packages/dsh-plugin/dist/client.js` и `client.css`).
3. Проверяет first-party WASM-пакет `workflows/solo-default` собранным бинарником
   `runlark plugin validate`.
4. Упаковывает автономный архив и контрольную сумму в `dist/`:
   - `dist/runlark-<version>-<target>.tar.gz`
   - `dist/runlark-<version>-<target>.tar.gz.sha256`

Для быстрой локальной проверки без release-оптимизации:

```sh
mise exec -- just package --profile debug
```

### Состав архива и проверка целостности

После распаковки `runlark-<version>-<target>.tar.gz` архив не требует исходников репозитория:

```text
runlark-<version>-<target>/
├── LICENSE
├── README.md
├── RUNNING.md
├── manifest.json
├── SHA256SUMS
├── bin/
│   ├── runlark
│   └── runlark-server
├── workflows/
│   └── solo-default/
├── dsh-plugin/
│   ├── package.json
│   ├── cordis.patch.yml
│   ├── patches/
│   ├── src/
│   └── dist/client.js
└── scripts/
    ├── setup-dsh-profile.mjs
    └── apply-dsh-patch.mjs
```

Проверка контрольных сумм перед запуском:

```sh
# macOS
shasum -a 256 -c runlark-0.1.0-aarch64-apple-darwin.tar.gz.sha256
tar -xzf runlark-0.1.0-aarch64-apple-darwin.tar.gz
cd runlark-0.1.0-aarch64-apple-darwin
shasum -a 256 -c SHA256SUMS

# Linux
sha256sum -c runlark-0.1.0-x86_64-unknown-linux-gnu.tar.gz.sha256
tar -xzf runlark-0.1.0-x86_64-unknown-linux-gnu.tar.gz
cd runlark-0.1.0-x86_64-unknown-linux-gnu
sha256sum -c SHA256SUMS
```

## 2. Локальный запуск solo-режима

### Вариант A: из распакованного release-архива

Перейдите в каталог распакованного архива (например, `~/runlark/runlark-0.1.0-<target>`):

```sh
# 1. Настроить DSH-профиль `runlark` (установит зависимости профиля и применит патч контроллера)
node ./scripts/setup-dsh-profile.mjs --profile runlark --daemon-url http://127.0.0.1:9876

# Проверить, что патч SessionCommandInterceptor применён
node ./scripts/apply-dsh-patch.mjs --target "${DSH_HOME:-$HOME/.dsh}/profiles/runlark" --check

# 2. Запустить DSH Web с профилем runlark (терминал 1)
dsh --profile runlark --port 3123
# DSH выведет URL вида: http://127.0.0.1:3123/?token=<LAUNCH_TOKEN>

# 3. Запустить локальный daemon Runlark (терминал 2)
# Передайте URL и токен через переменные окружения, чтобы не светить токен в аргументах процесса:
export RUNLARK_DSH_URL="http://127.0.0.1:3123"
export RUNLARK_DSH_TOKEN="<LAUNCH_TOKEN>"
./bin/runlark daemon --host 127.0.0.1 --port 9876 --data-dir "${HOME}/.local/share/runlark"
```

При запуске `bin/runlark daemon` из распакованного архива (даже из другой рабочей директории)
демон автоматически находит и устанавливает встроенный WASM-пакет `workflows/solo-default`
рядом с бинарником (`../workflows/solo-default`). Путь можно переопределить через
`RUNLARK_WORKFLOW_PACKAGE`.

### Вариант B: из исходников репозитория

```sh
scripts/setup
mise exec -- cargo build --bin runlark
mise exec -- node scripts/setup-dsh-profile.mjs --profile runlark --daemon-url http://127.0.0.1:9876

# Терминал 1: DSH Web
dsh --profile runlark --port 3123

# Терминал 2: Runlark daemon
export RUNLARK_DSH_URL="http://127.0.0.1:3123"
export RUNLARK_DSH_TOKEN="<LAUNCH_TOKEN>"
mise exec -- ./target/debug/runlark daemon --host 127.0.0.1 --port 9876 --data-dir .runlark
```

### Проверка solo-запуска и первый сценарий

```sh
# Проверка здоровья daemon и DSH bridge
curl -fsS http://127.0.0.1:9876/health
curl -fsS http://127.0.0.1:3123/api/runlark/health

# Проверка загруженного WASM workflow-пакета
./bin/runlark plugin list

# Регистрация локального репозитория и создание первой задачи
./bin/runlark project create my-app --repo-root /path/to/my-app --default-branch main
./bin/runlark task create "Добавить smoke-тест" --project-id <PROJECT_ID>
./bin/runlark task board

# Или через единый slash-интерфейс чата (работает и в DSH UI, и в CLI)
./bin/runlark chat send "/board"
```

После запуска откройте URL DSH Web в браузере и перейдите во вкладку **Runlark Board** в правой
панели. Закрытие вкладки браузера не останавливает выполнение задач: состояние хранится в SQLite
(`<data-dir>/runlark.db`, права каталога `0700` на Unix).

## 3. Self-host запуск `runlark-server` (команда 2–4 человека)

`runlark-server` реализует единый протокол совместной работы (SaaS и self-host) поверх
PostgreSQL и локального CAS-хранилища артефактов.

### Подготовка PostgreSQL и первичного владельца

```sh
export RUNLARK_POSTGRES_URL="postgres://runlark:<PASSWORD>@127.0.0.1:5432/runlark"

# Автоматически применит миграции и создаст первого owner-пользователя и workspace.
# Сохраните bearer_token из JSON-вывода в менеджер секретов (токен хранится на сервере только в виде SHA-256).
./bin/runlark-server bootstrap-owner \
  --user-id owner-1 \
  --display-name "Alice" \
  --email alice@example.com \
  --workspace-slug core-team \
  --workspace-name "Core Team"
```

### Запуск сервера

```sh
export RUNLARK_POSTGRES_URL="postgres://runlark:<PASSWORD>@127.0.0.1:5432/runlark"
export RUST_LOG="info"

./bin/runlark-server serve \
  --host 127.0.0.1 \
  --port 9880 \
  --cas-root /var/lib/runlark-server/cas \
  --runner-heartbeat-timeout-ms 30000
```

Проверка готовности:

```sh
curl -fsS http://127.0.0.1:9880/health
```

### Безопасность и эксплуатация self-host

- **TLS и reverse proxy:** `runlark-server` слушает plain HTTP и по умолчанию привязывается к
  `127.0.0.1:9880`. Для доступа команды поставьте перед ним reverse proxy с TLS (Caddy или nginx)
  и проксируйте запросы на `http://127.0.0.1:9880`.
- **Резервное копирование:** делайте бэкап **обеих** частей состояния:
  1. базы данных PostgreSQL (`pg_dump`);
  2. каталога `--cas-root` (хранит diff и evidence-артефакты по SHA-256).
- **Обращение с токенами:** передавайте `bearer_token` и invite-токены через переменные окружения
  (`RUNLARK_TEAM_TOKEN`, `RUNLARK_TEAM_INVITE_TOKEN`), `--token-file` или `--token-stdin`, а не
  через `--token` в командной строке.

Пример `systemd` unit для Linux (`/etc/systemd/system/runlark-server.service`):

```ini
[Unit]
Description=Runlark Team Collaboration Server
After=network-online.target postgresql.service
Wants=network-online.target

[Service]
Type=simple
User=runlark
Group=runlark
EnvironmentFile=/etc/runlark/server.env
ExecStart=/opt/runlark/bin/runlark-server serve --host 127.0.0.1 --port 9880 --cas-root /var/lib/runlark-server/cas
Restart=on-failure
RestartSec=5
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ReadWritePaths=/var/lib/runlark-server

[Install]
WantedBy=multi-user.target
```

Файл `/etc/runlark/server.env` (права `0600`, владелец `runlark`):

```sh
RUNLARK_POSTGRES_URL=postgres://runlark:<PASSWORD>@127.0.0.1:5432/runlark
RUST_LOG=info
```

### Подключение разработчиков и раннеров

На рабочей машине владельца (при запущенном локальном `runlark daemon`):

```sh
# 1. Авторизация на командном сервере (токен через stdin)
printf '%s' "${OWNER_TOKEN}" | ./bin/runlark team login \
  --server https://runlark.example.com \
  --workspace <WORKSPACE_ID> \
  --token-stdin

# 2. Перевод локального solo-проекта в командный (разрешён только при отсутствии активных runs)
./bin/runlark project promote-team <PROJECT_ID> \
  --server https://runlark.example.com \
  --workspace <WORKSPACE_ID>

# 3. Регистрация локального daemon как execution runner
./bin/runlark runner register --name "alice-macbook" --workspace <WORKSPACE_ID>

# 4. Приглашение участника команды
./bin/runlark team invite --workspace <WORKSPACE_ID> --invitee "Bob" --role member
```

На машине второго участника команды:

```sh
printf '%s' "${INVITE_TOKEN}" | ./bin/runlark team accept-invite \
  --server https://runlark.example.com \
  --name "Bob" \
  --token-stdin

./bin/runlark runner register --name "bob-linux" --workspace <WORKSPACE_ID>
./bin/runlark runner status --workspace <WORKSPACE_ID>
```

Полезные операционные команды A2:

- `runlark team sync` — ручной прогон синхронизации outbox, snapshot/events и CAS.
- `runlark team drafts` — просмотр offline-черновиков и конфликтных команд (`409`).
- `runlark runner assign <RUN_ID> --runner-id <RUNNER_ID>` — явное назначение `online`-раннера
  (автоматического takeover активного run нет).
- `runlark runner cancel-unreachable <RUN_ID> --reason "<причина>"` — явная отмена зависшего run
  на недоступном раннере.

## 4. Диагностика и восстановление

- **Рестарт daemon или DSH:** при старте `runlark daemon` автоматически запускает
  `ReconciliationEngine`: прерванные активные runs переводятся в `interrupted`, а ожидающие ответа
  человека (`WaitingForApproval`) и их вопросы сохраняются для повторного показа в DSH UI.
- **Патч DSH слетел после обновления зависимостей профиля:** выполните
  `node ./scripts/apply-dsh-patch.mjs --target "${DSH_HOME:-$HOME/.dsh}/profiles/runlark"`.
- **Офлайн-режим командного проекта:** при недоступности `runlark-server` авторитетные команды
  возвращают `503 team_server_offline` и не мутируют общую доску локально; черновики задач,
  комментариев и планов сохраняются локально и синхронизируются после восстановления связи.
- **Проверка WASM-пакета отдельно от daemon:**
  `./bin/runlark plugin validate ./workflows/solo-default --json`.
