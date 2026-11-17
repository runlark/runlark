# Changelog

## Unreleased

- Зафиксирован DSH spike (`deepseek-ai/deepseek-harness`, `dsh-v0.2.2-rc.3`) и спецификация перехватчика команд (`docs/dsh-spike.md`).
- Подключены архитектурные границы `easement.yml` (`--require-evidence`), реальные Rust/TS/SQLite/process-smoke гейты и вертикальный срез A0 (`runlark-domain`, `runlark-local`, `runlark-cli`, `packages/dsh-plugin`).
- Для организации runlark настроен отдельный приватный Sych App; current-head waiter ожидает `runlark-sych-reviewer[bot]`.
- Новый репозиторий публичный; CI и ожидание Sych перенесены на GitHub-hosted `ubuntu-24.04`.
- Git hook tests используют project-local mise pins и пустой global config на чистых hosted runners.
- Conventional Commits обязательны: commit-msg проверяет сообщение, pre-push проверяет все новые коммиты перед полным `just check`.
- Целевой GitHub-репозиторий нового проекта: `runlark/runlark`, отдельно от legacy `bondiano/runlark`.
- Новый local-first Runlark: DSH UI/runtime + Rust CLI/daemon + SQLite + WASM; optional collaboration.
- Сохранены исходный план, визуальный контракт и development/review guidance legacy.
- Инициализированы реальные bootstrap checks, security, tracked hooks и current-head Sych workflow.
- Продуктовые исходники требуют одновременного подключения compiler/architecture/test/smoke gates.
