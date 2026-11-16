set shell := ["bash", "-euo", "pipefail", "-c"]
set positional-arguments

default:
    @just --list

setup:
    scripts/setup

fmt:
    npm run format
    taplo fmt
    shfmt -w -i 4 scripts .githooks
    just --fmt --unstable

# Every source language needs its real compiler, architecture and behavior gates first.
lint-scope:
    node scripts/bootstrap-scope.mjs

lint-size:
    node scripts/file-limit.mjs

lint-files:
    npm run format:check
    npm run lint:markdown
    taplo fmt --check
    editorconfig-checker
    shfmt -d -i 4 scripts .githooks
    shellcheck $(shfmt -f scripts .githooks)
    actionlint
    zizmor --pedantic .github/workflows
    typos
    just --fmt --check --unstable
    git ls-files -z -- '*.md' ':!:.agents/skills/impeccable' ':!:.agents/skills/feod' ':!:.agents/skills/easement' | xargs -0 lychee --offline --include-fragments --no-progress

lint-skills:
    sh -n .agents/skills/impeccable/scripts/impeccable
    shellcheck --norc --shell=sh .agents/skills/impeccable/scripts/impeccable
    for source in .agents/skills/impeccable/scripts/*.js; do node --check "$source"; done
    node --check .agents/skills/feod/scripts/feod.mjs

security:
    npm run audit
    gitleaks dir . --redact --no-banner

lint: lint-scope lint-size lint-files lint-skills security

# Observable guard behavior against isolated real Git repositories.
test:
    node --test scripts/*.test.mjs

# The same complete bootstrap gate runs locally, in pre-push and in CI.
check: lint test

review *args="":
    scripts/wait-sych "$@"

impeccable *args="--help":
    .agents/skills/impeccable/scripts/impeccable "$@"
