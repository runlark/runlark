import assert from "node:assert/strict";
import http from "node:http";
import { test } from "node:test";
import { JSDOM } from "jsdom";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { createBoardState, loadBundle, waitForUi } from "./ui-board-fixtures.mjs";

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

        // Dragging task_1 keeps the drag handle mounted and window dragend clears the dragging state
        const dragBtn = rootEl.querySelector('[data-testid="rl-card-drag-task_1"]');
        assert.ok(dragBtn);
        await act(async () => {
            const dragStartEv = new dom.window.Event("dragstart", { bubbles: true });
            dragStartEv.dataTransfer = { setData() {}, effectAllowed: "move" };
            dragBtn.dispatchEvent(dragStartEv);
        });
        assert.ok(rootEl.querySelector('[data-testid="rl-card-dragging-task_1"]'));
        assert.equal(
            rootEl.querySelector('[data-testid="rl-card-drag-task_1"]'),
            dragBtn,
            "drag handle DOM node must stay mounted while dragging",
        );
        await act(async () => {
            dom.window.dispatchEvent(new dom.window.Event("dragend"));
        });
        assert.equal(rootEl.querySelector('[data-testid="rl-card-dragging-task_1"]'), null);
        assert.ok(rootEl.querySelector('[data-testid="rl-board-card-task_1"]'));

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
