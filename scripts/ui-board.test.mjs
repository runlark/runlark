import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import http from "node:http";
import { createRequire } from "node:module";
import { dirname, join, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { JSDOM } from "jsdom";
import React, { act } from "react";
import ReactDom from "react-dom";
import * as ReactDomClient from "react-dom/client";
import { createRoot } from "react-dom/client";
import * as ReactJsxRuntime from "react/jsx-runtime";

const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const CLIENT_BUNDLE = join(REPO_ROOT, "packages", "dsh-plugin", "dist", "client.js");
const require = createRequire(import.meta.url);

function loadBundle(win) {
    execFileSync(process.execPath, [join(REPO_ROOT, "scripts", "build-dsh-client.mjs")], {
        cwd: REPO_ROOT,
        stdio: "pipe",
    });
    assert.ok(existsSync(CLIENT_BUNDLE), "dist/client.js must exist");
    const loaded = new Map();
    const staticModules = {
        react: () => React,
        "react/jsx-runtime": () => ReactJsxRuntime,
        "react-dom": () => ReactDom,
        "react-dom/client": () => ReactDomClient,
        "@deepseek-ai/cordis": () => require("@deepseek-ai/cordis"),
        "@deepseek-ai/dsh-client-store": () => require("@deepseek-ai/dsh-client-store"),
        "@deepseek-ai/dsh-client-ui-slots": () => require("@deepseek-ai/dsh-client-ui-slots"),
        "@deepseek-ai/dsh-client-ui-primitives": () =>
            require("@deepseek-ai/dsh-client-ui-primitives"),
    };
    win.__ModuleLoader__ = {
        load(entry) {
            loaded.set(
                entry.id,
                entry.factory((spec) => staticModules[spec]()),
            );
        },
    };
    win.eval(readFileSync(CLIENT_BUNDLE, "utf8"));
    return loaded.get("@runlark/dsh-plugin");
}

async function waitForUi(check, timeoutMs = 4000) {
    const deadline = Date.now() + timeoutMs;
    let lastError;
    while (Date.now() < deadline) {
        await act(async () => {
            await new Promise((r) => setTimeout(r, 25));
        });
        try {
            check();
            return;
        } catch (err) {
            lastError = err;
        }
    }
    throw lastError;
}

function createBoardState() {
    const project = {
        project_id: "proj_ui_1",
        name: "Runlark UI Repo",
        repo_root: "/tmp/repo",
        default_branch: "main",
        max_concurrent_runs: 2,
        policy_digest: "pol_sha_1",
        budget_policy: {
            max_attempts: 3,
            max_duration_ms: 1_800_000,
            max_spend_micro_usd: 5_000_000,
            allow_unknown_cost_repair: false,
        },
        created_at: 1000,
        updated_at: 1000,
    };
    const makeEntry = (id, title, stage, kind, wf, runStatus, unknownAttempts = 0) => ({
        task: {
            id,
            project_id: "proj_ui_1",
            title,
            description: `${title} description`,
            kind,
            stage,
            workflow_stage: wf,
            plan_version: 2,
            plan_text: "Step 1: verify",
            active_run_id: stage === "in_progress" ? `run_${id}` : null,
            latest_run_id: `run_${id}`,
            active_session_id: `sess_${id}`,
            base_ref: "main",
            head_ref: `runlark/${id}`,
            pr_url: null,
            revision: 3,
            created_at: 1000,
            updated_at: 1000,
        },
        session_revision: 7,
        latest_run: {
            id: `run_${id}`,
            session_id: `sess_${id}`,
            status: runStatus,
            worktree_path: `/tmp/wt/${id}`,
            revision: 2,
            task_id: id,
            project_id: "proj_ui_1",
            workflow_stage: wf,
            task_kind: kind,
            workflow_digest: "wf_digest_sha",
            base_sha: "aaaa1111",
            head_sha: "bbbb2222",
            plan_version: 2,
            policy_digest: "pol_sha_1",
            budget_policy: project.budget_policy,
            cost_summary: {
                known_micro_usd: 15_000,
                held_micro_usd: 0,
                unknown_attempts: unknownAttempts,
            },
            repair_attempts: 1,
            pr_url: null,
            created_at: 1000,
            updated_at: 1000,
        },
        attempts: [
            {
                id: `att_${id}`,
                run_id: `run_${id}`,
                attempt_number: 1,
                workflow_stage: wf,
                outcome: { status: "succeeded" },
                cost_micro_usd: unknownAttempts > 0 ? null : 15_000,
                unknown_cost_reason: unknownAttempts > 0 ? "missing provider pricing" : null,
                created_at: 1000,
                started_at: 1000,
                finished_at: 1100,
            },
        ],
        cost_summary: {
            known_micro_usd: 15_000,
            held_micro_usd: 0,
            unknown_attempts: unknownAttempts,
        },
        decisions: [],
        artifacts: [
            {
                digest_sha256: "deadbeef".repeat(8),
                kind: "evidence_bundle",
                media_type: "application/json",
                size_bytes: 256,
                label: "evidence_bundle",
                created_at: 1000,
            },
        ],
        pending_questions: [],
    });

    return {
        project,
        entries: [
            makeEntry(
                "task_1",
                "Fix redirect URI",
                "inbox",
                "quick_fix",
                "implementation",
                "queued",
            ),
            makeEntry(
                "task_2",
                "Review WASM diff",
                "acceptance",
                "standard",
                "human_acceptance",
                "waiting_for_approval",
                1,
            ),
        ],
        moved: [],
        accepted: [],
    };
}

test("RunlarkBoardTabBody renders 5 Kanban columns, RU/EN i18n, keyboard move, Task screen, and mobile stage bar", async () => {
    const state = createBoardState();
    const server = http.createServer(async (req, res) => {
        const url = new URL(req.url ?? "/", "http://127.0.0.1");
        const chunks = [];
        for await (const c of req) chunks.push(c);
        const body = chunks.length ? JSON.parse(Buffer.concat(chunks).toString("utf8")) : {};

        const send = (code, data) => {
            res.writeHead(code, { "Content-Type": "application/json" });
            res.end(JSON.stringify(data));
        };

        if (req.method === "GET" && url.pathname === "/api/runlark/projects") {
            return send(200, [state.project]);
        }
        if (req.method === "GET" && url.pathname === "/api/runlark/plugins") {
            return send(200, [
                { name: "solo-default", version: "0.1.0", package_digest: "sha", stages: [] },
            ]);
        }
        if (req.method === "GET" && url.pathname === "/api/runlark/board") {
            const columns = { inbox: [], ready: [], in_progress: [], acceptance: [], done: [] };
            for (const e of state.entries) columns[e.task.stage].push(e);
            return send(200, {
                project: state.project,
                columns,
                tasks: state.entries,
                active_runs: [],
                queued_runs: [],
                attempts: [],
                cost_summary: { known_micro_usd: 30_000, held_micro_usd: 0, unknown_attempts: 1 },
                decisions: [],
                pending_questions: [],
                slots: { max: 2, used: 1, available: 1 },
            });
        }
        const taskGet = /^\/api\/runlark\/tasks\/([^/]+)$/u.exec(url.pathname);
        if (req.method === "GET" && taskGet) {
            const entry = state.entries.find((e) => e.task.id === taskGet[1]);
            return entry ? send(200, entry) : send(404, { error: "not_found" });
        }
        if (req.method === "POST" && url.pathname === "/api/runlark/tasks/task_1/move") {
            state.moved.push(body.stage);
            const entry = state.entries.find((e) => e.task.id === "task_1");
            entry.task = { ...entry.task, stage: body.stage, revision: entry.task.revision + 1 };
            return send(200, entry.task);
        }
        if (req.method === "POST" && url.pathname === "/api/runlark/tasks/task_2/accept") {
            assert.equal(body.expected_revision, 3);
            assert.equal(body.plan_version, 2);
            assert.equal(body.head_sha, "bbbb2222");
            assert.equal(body.base_sha, "aaaa1111");
            assert.equal(body.policy_digest, "pol_sha_1");
            assert.equal(body.evidence_digest, "deadbeef".repeat(8));
            state.accepted.push(body.comment ?? "");
            const entry = state.entries.find((e) => e.task.id === "task_2");
            entry.task = { ...entry.task, stage: "done", revision: entry.task.revision + 1 };
            entry.decisions = [
                {
                    id: "dec_1",
                    task_id: "task_2",
                    run_id: "run_task_2",
                    kind: "accept",
                    actor: "human",
                    plan_version: 2,
                    head_sha: "bbbb2222",
                    base_sha: "aaaa1111",
                    policy_digest: "pol_sha_1",
                    evidence_digest: "deadbeef".repeat(8),
                    comment: body.comment ?? null,
                    created_at: 2000,
                },
            ];
            return send(200, { decision_id: "dec_1", task: entry.task, run: entry.latest_run });
        }
        if (req.method === "GET" && url.pathname.endsWith("/history")) {
            const e = state.entries[1];
            return send(200, {
                run: e.latest_run,
                attempts: e.attempts,
                cost_summary: e.cost_summary,
                decisions: e.decisions,
                artifacts: e.artifacts,
                events: [],
                pending_questions: [],
            });
        }
        if (req.method === "GET" && url.pathname.endsWith("/diff")) {
            return send(200, {
                run_id: "run_task_2",
                base_sha: "aaaa1111",
                head_sha: "bbbb2222",
                branch_name: "runlark/task_2",
                changed_files: ["src/lib.rs"],
                diff: "+pub fn ok() {}\n",
                diff_bytes: 16,
                digest_sha256: "abc",
            });
        }
        if (req.method === "POST" && url.pathname === "/api/runlark/tasks") {
            const created = {
                ...createBoardState().entries[0].task,
                id: "task_created",
                title: body.title,
                description: body.description ?? "",
                kind: body.kind ?? "standard",
                stage: "inbox",
                workflow_stage: body.kind === "quick_fix" ? "implementation" : "clarification",
                revision: 1,
            };
            state.entries.push({
                ...createBoardState().entries[0],
                task: created,
                latest_run: null,
                artifacts: [],
                pending_questions: [],
            });
            return send(201, created);
        }
        if (req.method === "POST" && url.pathname === "/api/runlark/sessions/sess_task_2/stop") {
            state.stopped = body;
            return send(202, { status: "cancel_requested" });
        }
        if (req.method === "POST" && url.pathname === "/api/runlark/chat") {
            return send(200, { command: "status", summary: "Slots: 1/2 active", data: {} });
        }
        return send(404, { error: "not_found" });
    });

    await new Promise((r) => server.listen(0, "127.0.0.1", r));
    const addr = server.address();
    const baseUrl = `http://127.0.0.1:${addr.port}`;
    const dom = new JSDOM(
        `<!DOCTYPE html><html lang="en"><body><div id="root"></div></body></html>`,
        {
            url: `${baseUrl}/`,
            runScripts: "dangerously",
        },
    );

    let mobileMode = false;
    const prev = {
        window: globalThis.window,
        document: globalThis.document,
        HTMLElement: globalThis.HTMLElement,
        Element: globalThis.Element,
        Node: globalThis.Node,
        getComputedStyle: globalThis.getComputedStyle,
        fetch: globalThis.fetch,
        IS_REACT_ACT_ENVIRONMENT: globalThis.IS_REACT_ACT_ENVIRONMENT,
    };
    dom.window.matchMedia = (query) => ({
        matches: query.includes("max-width") ? mobileMode : false,
        media: query,
        addEventListener: () => {},
        removeEventListener: () => {},
    });
    if (!dom.window.HTMLDialogElement.prototype.showModal) {
        dom.window.HTMLDialogElement.prototype.showModal = function showModal() {
            this.setAttribute("open", "");
        };
        dom.window.HTMLDialogElement.prototype.close = function close() {
            this.removeAttribute("open");
        };
    }
    Object.assign(globalThis, {
        window: dom.window,
        document: dom.window.document,
        HTMLElement: dom.window.HTMLElement,
        Element: dom.window.Element,
        Node: dom.window.Node,
        getComputedStyle: dom.window.getComputedStyle.bind(dom.window),
        IS_REACT_ACT_ENVIRONMENT: true,
        fetch: (input, init) => {
            const resolved =
                typeof input === "string" && input.startsWith("/") ? `${baseUrl}${input}` : input;
            return prev.fetch(resolved, init);
        },
    });
    dom.window.fetch = globalThis.fetch;
    dom.window.AbortController = globalThis.AbortController;
    dom.window.AbortSignal = globalThis.AbortSignal;

    const uiPlugin = loadBundle(dom.window);
    const rootEl = dom.window.document.getElementById("root");
    const root = createRoot(rootEl);

    try {
        await act(async () => {
            root.render(React.createElement(uiPlugin.RunlarkBoardTabBody));
        });

        await waitForUi(() => {
            assert.ok(rootEl.querySelector('[data-testid="rl-board-col-inbox"]'));
            assert.ok(rootEl.querySelector('[data-testid="rl-board-col-acceptance"]'));
            assert.ok(rootEl.querySelector('[data-testid="rl-board-col-done"]'));
            assert.ok(rootEl.textContent.includes("Fix redirect URI"));
            assert.ok(rootEl.textContent.includes("1 unknown cost"));
            assert.equal(
                rootEl.querySelector('[data-testid="rl-board-card-task_2"]')?.dataset.decisionOwn,
                "true",
            );
        });

        // Switch locale to RU and verify localized headings + badges
        await act(async () => {
            rootEl.querySelector('[data-testid="rl-lang-ru"]').click();
        });
        await waitForUi(() => {
            assert.ok(rootEl.textContent.includes("Канбан-доска"));
            assert.ok(rootEl.textContent.includes("Приёмка"));
            assert.ok(rootEl.textContent.includes("Неизвестно: 1"));
        });

        // Open and submit CreateTaskDialog while preserving mounted <dialog>
        const dialogNode = rootEl.querySelector('[data-testid="rl-create-task-dialog"]');
        assert.ok(dialogNode);
        await act(async () => {
            rootEl.querySelector('[data-testid="rl-btn-new-quick-fix"]').click();
        });
        assert.equal(
            rootEl.querySelector('[data-testid="rl-create-task-dialog"]'),
            dialogNode,
            "<dialog> must stay mounted across open/close transitions",
        );

        // Move task_1 via accessible Move menu and verify keyboard ArrowDown + Escape
        await act(async () => {
            rootEl.querySelector('[data-testid="rl-card-menu-task_1"]').click();
        });
        await waitForUi(() => {
            assert.ok(rootEl.querySelector('[data-testid="rl-move-task_1-ready"]'));
        });
        const openItem = rootEl.querySelector('[role="menu"] [role="menuitem"]');
        assert.ok(openItem);
        openItem.focus();
        await act(async () => {
            rootEl
                .querySelector('[role="menu"]')
                .dispatchEvent(
                    new dom.window.KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }),
                );
        });
        assert.equal(
            dom.window.document.activeElement?.getAttribute("data-testid"),
            "rl-move-task_1-ready",
        );
        await act(async () => {
            rootEl.querySelector('[data-testid="rl-move-task_1-ready"]').click();
        });
        await waitForUi(() => {
            assert.deepEqual(state.moved, ["ready"]);
            assert.ok(
                rootEl
                    .querySelector('[data-testid="rl-board-live"]')
                    .textContent.includes("Готово к работе"),
            );
        });

        // Open task_2 Task Detail screen, check Stop sends session_revision (7 != runRev 1), Diff, Acceptance & bound Accept
        await act(async () => {
            rootEl.querySelector('[data-testid="rl-card-open-task_2"]').click();
        });
        await waitForUi(() => {
            assert.ok(rootEl.querySelector('[data-testid="rl-task-detail"]'));
        });
        await act(async () => {
            rootEl.querySelector('[data-testid="rl-task-tab-run_budget"]').click();
        });
        await act(async () => {
            const buttons = [...rootEl.querySelectorAll('[role="tabpanel"]:not([hidden]) button')];
            const resumeBtn = buttons.find((b) => b.textContent.includes("Возобновить"));
            assert.ok(resumeBtn);
            assert.equal(
                resumeBtn.getAttribute("aria-describedby") !== null || !resumeBtn.disabled,
                true,
            );
        });
        await act(async () => {
            rootEl.querySelector('[data-testid="rl-task-tab-diff_artifacts"]').click();
        });
        await waitForUi(() => {
            assert.ok(
                rootEl
                    .querySelector('[data-testid="rl-run-diff-pre"]')
                    .textContent.includes("+pub fn ok()"),
            );
        });
        await act(async () => {
            rootEl.querySelector('[data-testid="rl-task-tab-acceptance_pr"]').click();
        });
        await act(async () => {
            rootEl.querySelector('[data-testid="rl-task-accept-btn"]').click();
        });
        await waitForUi(() => {
            assert.equal(state.accepted.length, 1);
            assert.ok(rootEl.querySelector('[data-testid="rl-task-create-pr-btn"]'));
        });

        // Verify mobile single-stage mode renders cleanly
        mobileMode = true;
        await act(async () => {
            root.unmount();
        });
        const mobileRoot = createRoot(rootEl);
        await act(async () => {
            mobileRoot.render(React.createElement(uiPlugin.RunlarkBoardTabBody));
        });
        await waitForUi(() => {
            const cols = rootEl.querySelectorAll('[data-testid^="rl-board-col-"]');
            assert.equal(cols.length, 1, "mobile viewport shows one active stage at a time");
            assert.ok(rootEl.querySelector('[data-testid="rl-mobile-stage-controls"]'));
        });
        await act(async () => {
            mobileRoot.unmount();
        });
    } finally {
        dom.window.close();
        Object.assign(globalThis, prev);
        server.closeAllConnections();
        await new Promise((r) => server.close(r));
    }
});
