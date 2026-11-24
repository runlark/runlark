# Runlark

Runlark is a board for software work that agents do and people accept. This glossary is the one language
shared by the product, its code, its docs and its issues. Russian labels are the UI wording.

## Work planning

**Task**:
The unit of work whose result a person accepts. A task outlives the runs, sessions and agents that work
on it. RU: «Задача».
_Avoid_: ticket, card, job

**Quick fix**:
A task kind that skips clarification and planning. RU: «Быстрый фикс».
_Avoid_: hotfix, small task

**Plan**:
The agreed approach for a task. Every change produces a new plan version. RU: «План».
_Avoid_: spec, design

**Project**:
A registered repository with its default branch, run limits and policy. RU: «Проект».
_Avoid_: workspace, repo

**Column**:
A group on the board computed from a task's lifecycle and its current run: Inbox, Ready, In progress,
Acceptance, Done. It is never set directly; moving a card issues a command. RU: «Входящие», «Готово к
работе», «В работе», «Приёмка», «Завершено».
_Avoid_: status, stage, lane

## Execution

**Run**:
One execution of a task's workflow, pinned when it starts to a plan version, a workflow package, a policy
and a base commit. RU: «Запуск».
_Avoid_: job, session, attempt

**Stage**:
A phase of a workflow that a run passes through: clarification, plan, implementation, verification,
decision. RU: «Стадия»; the decision stage is «Решение человека».
_Avoid_: column, status, phase

**Step**:
One visit of a run to a stage. Going back from verification to implementation starts a new step.
_Avoid_: stage, attempt, job

**Session**:
The agent conversation attached to a task. Every run of the task continues the same session.
RU: «Сессия».
_Avoid_: agent session, chat, thread; never a sign-in

**Attempt**:
One agent turn within a run; the unit that carries cost. RU: «Попытка».
_Avoid_: try, iteration, turn

**Repair**:
An attempt that goes back from verification to implementation to fix failed checks. Repairs have their
own limit. RU: «Попытка исправления».
_Avoid_: retry, fix loop

**Question**:
Something the agent asks a person during a run. It stays open until it is answered or the run ends.
RU: «Вопрос».
_Avoid_: approval, prompt

**Waiting for answer**:
A run paused on an open question. It holds no slot. RU: «Ждёт ответа».
_Avoid_: waiting for approval

**Waiting for acceptance**:
A run whose result or plan awaits a person's decision. It holds no slot. RU: «Ждёт решения».
_Avoid_: waiting for approval

**Slot**:
A unit of execution capacity. A run holds one while its agent may be working: while it is running, or
cancel requested during a turn. RU: «Слот».
_Avoid_: worker, lane

**Queue**:
The runs waiting for a slot, in arrival order. A run that comes back from waiting rejoins the queue.
RU: «Очередь».
_Avoid_: backlog

**Admission**:
The only way a run starts running: it takes a free slot and its own worktree in queue order.
_Avoid_: start, dispatch

**Worktree**:
A Git checkout that belongs to exactly one run. RU: «Worktree».
_Avoid_: workspace, checkout, sandbox

**Cancel requested**:
A stop that has been asked for but not yet confirmed. It survives restarts. RU: «Остановка запрошена».
_Avoid_: stopped, cancelled

**Stopped**:
A run whose stop was confirmed by the agent, or marked by a person as stopped with an unknown outcome.
Never set by a timer. RU: «Остановлен».
_Avoid_: cancelled, killed

**Interrupted**:
A run that Runlark lost sight of because Runlark or the agent restarted. Its outcome is unknown until
reconciliation. RU: «Прерван · неизвестно».
_Avoid_: stopped, failed, crashed

**Unknown**:
An outcome, delivery or cost that has not been reconciled. Never treated as zero or as success.
RU: «Неизвестно».
_Avoid_: pending, zero, n/a

**Reconciliation**:
Comparing recorded intent with what actually happened, after a restart or a lost confirmation.
_Avoid_: sync, recovery

**Quarantined run**:
A run whose stored state Runlark cannot read. People see it; it is never admitted and it counts as
holding a slot. RU: «Карантин».
_Avoid_: corrupted, broken

**Budget**:
The limits on what a run may consume: turns, repairs, time and tokens. Its workflow declares them; a
project may only tighten them. RU: «Бюджет».
_Avoid_: quota, policy, spend

**Quota**:
The remaining share of a subscription's usage windows. It belongs to an account, not to a run, and may be
unknown. A run is not admitted while its account's quota is exhausted. RU: «Квота».
_Avoid_: budget, credits, limit

**Run report**:
The structured statement a run makes when it starts (what it will do) and when it finishes (what it did
and what the checks showed), in the shape its workflow package requires. A run without a valid final
report does not reach acceptance. RU: «Отчёт запуска».
_Avoid_: summary, log

## Review and acceptance

**Decision**:
A person's recorded verdict on a run: accept, request changes or approve plan. Only a human makes
decisions. RU: «Решение».
_Avoid_: approval, review, vote

**Accept**:
The decision that a run's result becomes the task's result. It is the only way a task is completed, and
a completed task is never reopened. RU: «Принять результат».
_Avoid_: approve, merge, done

**Request changes**:
The decision that sends a result under acceptance back for more work. RU: «Вернуть на доработку».
_Avoid_: reject, decline

**Approve plan**:
The decision that fixes one plan version as the one to implement. RU: «Одобрить план».
_Avoid_: plan accept, sign-off

**Decision binding**:
The exact versions a decision refers to: plan version, head and base commits, policy and evidence
manifest. A decision with an incomplete binding is invalid.
_Avoid_: context, snapshot

**Check**:
A verification that a workflow declares, such as tests or a review. Running it produces an evidence item.
RU: «Проверка».
_Avoid_: step, gate, job

**Evidence item**:
One typed fact about a result at a head commit — a diff, check, test, review, report, cost or log — with
an outcome of pass, fail or unknown. RU: «Доказательство».
_Avoid_: artifact, attachment

**Evidence manifest**:
The canonical list of a run's evidence items for one head commit. Its digest is what a decision binds.
_Avoid_: bundle, archive

**Publish receipt**:
The record that a result was published as a pull request. It is not evidence.
_Avoid_: evidence, PR artifact

## Commands

**Command**:
A request to change Runlark's state, with a stable ID and an immutable body. The same ID with a
different body is a conflict; repeating a command returns its original result.
_Avoid_: action, request, message

**Intent**:
A command that is recorded but whose effect is not yet confirmed.
_Avoid_: job, pending action

## Actors and agents

**Actor**:
Who performed an action: a human, an agent, a runner or the system. Only a human makes decisions.
_Avoid_: user, author

**Agent**:
The AI that acts within a session.
_Avoid_: runner, bot, harness

**Harness**:
An external agent program that Runlark drives over ACP, such as Codex, opencode or omp.
_Avoid_: agent, runtime, plugin

**Runner**:
A machine that executes runs on behalf of a team.
_Avoid_: agent, worker

## Workflows

**Workflow**:
A versioned package that defines a task's stages, checks, budget and report shape. A run is pinned to one
workflow version. RU: «Процесс».
_Avoid_: plugin, module, pipeline

## Collaboration

**Team**:
A shared space where members with roles work on projects together. RU: «Команда».
_Avoid_: workspace, organization

**Promotion**:
Handing a solo project over to a team after the team server acknowledges it.
_Avoid_: migration, upload

**Offline draft**:
A team change kept locally until it is synced or resolved.
_Avoid_: pending change

**Lease epoch**:
The fencing number of the runner that currently owns a run.
_Avoid_: lock, version
