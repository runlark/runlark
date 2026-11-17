import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import http from "node:http";
import { createRequire } from "node:module";
import { dirname, join, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { Context as CordisContext } from "@deepseek-ai/cordis";
import { JSDOM } from "jsdom";
import React, { act } from "react";
import ReactDom from "react-dom";
import * as ReactDomClient from "react-dom/client";
import { createRoot } from "react-dom/client";
import * as ReactJsxRuntime from "react/jsx-runtime";

const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const CLIENT_BUNDLE = join(REPO_ROOT, "packages", "dsh-plugin", "dist", "client.js");
const require = createRequire(import.meta.url);

function loadCompiledDshClientBundle(win) {
    execFileSync(process.execPath, [join(REPO_ROOT, "scripts", "build-dsh-client.mjs")], {
        cwd: REPO_ROOT,
        stdio: "pipe",
    });
    assert.ok(existsSync(CLIENT_BUNDLE), "dist/client.js must be built");

    const loadedModules = new Map();
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
            const exportsObj = entry.factory((specifier) => {
                const loader = staticModules[specifier];
                if (loader) {
                    return loader();
                }
                throw new Error(`Unexpected external require in client bundle: ${specifier}`);
            });
            loadedModules.set(entry.id, exportsObj);
        },
    };

    const bundleCode = readFileSync(CLIENT_BUNDLE, "utf8");
    win.eval(bundleCode);
    return loadedModules.get("@runlark/dsh-plugin");
}

async function waitForCondition(check, timeoutMs = 4000) {
    const deadline = Date.now() + timeoutMs;
    let lastError;
    while (Date.now() < deadline) {
        await act(async () => {
            await new Promise((r) => setTimeout(r, 25));
        });
        try {
            check();
            return;
        } catch (error) {
            lastError = error;
        }
    }
    throw lastError;
}

test("compiled DSH client bundle mounts right-sidebar card, streams live events, and restores WaitingForApproval across restart", async () => {
    const state = {
        session: {
            session_id: "sess_card_1",
            task_id: "task_card_1",
            cwd: "/tmp/wt_card",
            status: "active",
            current_revision: 3,
            active_run_id: "run_card_1",
            active_run_status: "waiting_for_approval",
            worktree_path: "/tmp/wt_card_wt",
            latest_run_id: "run_card_1",
            latest_run_status: "waiting_for_approval",
            latest_worktree_path: "/tmp/wt_card_wt",
            dispatched_commands_count: 1,
            pending_question: {
                question_id: "q_card_1",
                session_id: "sess_card_1",
                run_id: "run_card_1",
                prompt: "Deploy migration to SQLite?",
                options: ["Approve migration", "Hold"],
                status: "pending",
                answer: null,
                created_at: 1000,
                updated_at: 1000,
            },
            created_at: 1000,
            updated_at: 1000,
        },
        events: [
            {
                run_id: "run_card_1",
                seq: 0,
                event_type: "PromptEnqueued",
                payload: { PromptEnqueued: { prompt: "Migrate schema" } },
                created_at: 1000,
            },
            {
                run_id: "run_card_1",
                seq: 1,
                event_type: "QuestionAsked",
                payload: { QuestionAsked: { prompt: "Deploy migration to SQLite?" } },
                created_at: 1010,
            },
        ],
        answeredCommandIds: [],
    };

    const server = http.createServer(async (req, res) => {
        const url = new URL(req.url ?? "/", "http://127.0.0.1");
        const chunks = [];
        for await (const c of req) chunks.push(c);
        const bodyText = Buffer.concat(chunks).toString("utf8");
        const json = bodyText ? JSON.parse(bodyText) : {};

        if (req.method === "GET" && url.pathname === "/api/runlark/sessions/sess_card_1") {
            res.writeHead(200, { "Content-Type": "application/json" });
            res.end(JSON.stringify(state.session));
            return;
        }
        if (
            req.method === "GET" &&
            url.pathname === "/api/runlark/sessions/sess_card_1/questions"
        ) {
            res.writeHead(200, { "Content-Type": "application/json" });
            res.end(
                JSON.stringify(
                    state.session.pending_question ? [state.session.pending_question] : [],
                ),
            );
            return;
        }
        if (
            req.method === "GET" &&
            (url.pathname === "/api/runlark/sessions/sess_card_1/events" ||
                url.pathname === "/api/runlark/runs/run_card_1/events")
        ) {
            const afterSeq = url.searchParams.get("after_seq");
            const minSeq = afterSeq === null ? -1 : Number(afterSeq);
            const filtered = state.events.filter((e) => e.seq > minSeq);
            res.writeHead(200, { "Content-Type": "application/json" });
            res.end(JSON.stringify(filtered));
            return;
        }
        if (req.method === "POST" && url.pathname === "/api/runlark/questions/q_card_1/answer") {
            state.answeredCommandIds.push(String(json.command_id));
            state.session = {
                ...state.session,
                current_revision: 4,
                active_run_status: "running",
                latest_run_status: "running",
                dispatched_commands_count: 0,
                pending_question: null,
            };
            state.events.push({
                run_id: "run_card_1",
                seq: 2,
                event_type: "QuestionAnswered",
                payload: { QuestionAnswered: { answer: json.answer } },
                created_at: 1020,
            });
            res.writeHead(201, { "Content-Type": "application/json" });
            res.end(
                JSON.stringify({
                    command_id: json.command_id,
                    status: "pending",
                    question_id: "q_card_1",
                    current_revision: 4,
                    run_status: "running",
                }),
            );
            return;
        }
        if (req.method === "POST" && url.pathname === "/api/runlark/sessions/sess_card_1/stop") {
            state.session = {
                ...state.session,
                current_revision: 5,
                active_run_status: "cancel_requested",
                latest_run_status: "cancel_requested",
            };
            state.events.push({
                run_id: "run_card_1",
                seq: 3,
                event_type: "RunCancelRequested",
                payload: { RunCancelRequested: { reason: "user" } },
                created_at: 1030,
            });
            res.writeHead(200, { "Content-Type": "application/json" });
            res.end(JSON.stringify({ accepted: true }));
            return;
        }

        res.writeHead(404);
        res.end("not found");
    });

    await new Promise((resolveListen) => server.listen(0, "127.0.0.1", resolveListen));
    const addr = server.address();
    assert.ok(addr && typeof addr === "object");
    const baseUrl = `http://127.0.0.1:${addr.port}`;

    const dom = new JSDOM(
        `<!DOCTYPE html><html lang="en"><body><div id="root"></div></body></html>`,
        {
            url: `${baseUrl}/`,
            runScripts: "dangerously",
        },
    );

    const prevGlobals = {
        window: globalThis.window,
        document: globalThis.document,
        HTMLElement: globalThis.HTMLElement,
        Element: globalThis.Element,
        Node: globalThis.Node,
        getComputedStyle: globalThis.getComputedStyle,
        fetch: globalThis.fetch,
        IS_REACT_ACT_ENVIRONMENT: globalThis.IS_REACT_ACT_ENVIRONMENT,
    };

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
            return prevGlobals.fetch(resolved, init);
        },
    });
    dom.window.fetch = globalThis.fetch;
    dom.window.AbortController = globalThis.AbortController;

    try {
        const clientPlugin = loadCompiledDshClientBundle(dom.window);
        assert.equal(clientPlugin.RUNLARK_TAB_ID, "@runlark/dsh-plugin");
        assert.equal(clientPlugin.RUNLARK_TAB_KIND, "runlark");

        const cordis = new CordisContext();
        const registeredTabs = [];
        const registeredSlots = [];
        const openedTabs = [];

        cordis.provide("sidebarRightTabs", {
            register(def) {
                registeredTabs.push(def);
                return () => {};
            },
        });
        cordis.provide("sidebarRight", {
            openTab(kind) {
                openedTabs.push(kind);
            },
        });
        cordis.provide("slots", {
            register(spec, component) {
                registeredSlots.push({ ...spec, component });
                return () => {};
            },
            inject(_name, fn) {
                return fn();
            },
        });
        cordis.provide("uiSession", {
            adapter: {
                current: {
                    getSnapshot: () => ({ key: "sess_card_1" }),
                    subscribe: () => () => {},
                },
            },
        });

        await cordis.plugin(clientPlugin);
        assert.equal(registeredTabs.length, 1);
        assert.ok(registeredSlots.some((s) => s.name === "sidebar.right.pane.tab"));
        assert.ok(registeredSlots.some((s) => s.name === "sidebar.right.pane.tab.title"));

        await waitForCondition(() => {
            assert.ok(
                openedTabs.includes("runlark"),
                "Pending approval must auto-open Runlark tab",
            );
        });

        const rootEl = dom.window.document.getElementById("root");
        const root = createRoot(rootEl);

        const renderBody = async (width) => {
            dom.window.innerWidth = width;
            const Body = clientPlugin.RunlarkTabBody;
            await act(async () => {
                root.render(
                    React.createElement(Body, {
                        sessionId: "sess_card_1",
                        useSessions: (selector) =>
                            selector({ byId: { sess_card_1: { cwd: "/tmp/wt_card" } } }),
                        useTabInfo: () => ({ tab: { title: "Runlark" } }),
                    }),
                );
            });
        };

        await renderBody(1280);

        await waitForCondition(() => {
            assert.equal(
                rootEl.querySelector('[data-testid="rl-session-id"]')?.textContent,
                "sess_card_1",
            );
            assert.equal(
                rootEl.querySelector('[data-testid="rl-task-id"]')?.textContent,
                "task_card_1",
            );
            assert.equal(
                rootEl.querySelector('[data-testid="rl-run-id"]')?.textContent,
                "run_card_1",
            );
            assert.equal(
                rootEl.querySelector('[data-testid="rl-worktree-path"]')?.textContent,
                "/tmp/wt_card_wt",
            );
            assert.ok(
                rootEl.querySelector('[data-testid="rl-revision"]')?.textContent?.includes("rev 3"),
            );
            assert.ok(rootEl.querySelector('[data-run-status="waiting_for_approval"]'));
            assert.ok(
                rootEl.querySelector('[data-tone="unknown"]'),
                "unknown delivery badge must be slate",
            );
            assert.ok(rootEl.querySelector('[data-testid="rl-question-card"]'));
            assert.equal(rootEl.querySelectorAll("[data-event-seq]").length, 2);
        });

        const styleEl = dom.window.document.querySelector(
            'style[data-plugin="@runlark/dsh-plugin"]',
        );
        assert.ok(styleEl && styleEl.textContent?.includes(".rl-workbench-root"));

        await renderBody(375);
        assert.ok(rootEl.querySelector('[data-testid="rl-question-card"]'));

        const form = rootEl.querySelector('[data-testid="rl-question-card"] form');
        await act(async () => {
            form.dispatchEvent(new dom.window.Event("submit", { bubbles: true, cancelable: true }));
        });

        await waitForCondition(() => {
            assert.ok(rootEl.querySelector('[data-run-status="running"]'));
            assert.equal(rootEl.querySelector('[data-testid="rl-question-card"]'), null);
            assert.equal(rootEl.querySelectorAll("[data-event-seq]").length, 3);
        });
        assert.equal(state.answeredCommandIds.length, 1);

        const stopBtn = rootEl.querySelector('[data-testid="rl-btn-stop"]');
        await act(async () => {
            stopBtn.click();
        });

        await waitForCondition(() => {
            assert.ok(rootEl.querySelector('[data-run-status="cancel_requested"]'));
            assert.equal(rootEl.querySelector('[data-run-status="stopped"]'), null);
            assert.equal(rootEl.querySelectorAll("[data-event-seq]").length, 4);
        });

        state.session = {
            ...state.session,
            current_revision: 6,
            active_run_id: null,
            active_run_status: null,
            latest_run_status: "stopped",
        };
        state.events.push({
            run_id: "run_card_1",
            seq: 4,
            event_type: "RunStopped",
            payload: { RunStopped: {} },
            created_at: 1040,
        });

        await waitForCondition(() => {
            assert.ok(rootEl.querySelector('[data-run-status="stopped"]'));
            assert.equal(rootEl.querySelectorAll("[data-event-seq]").length, 5);
        });

        state.session = {
            ...state.session,
            active_run_id: "run_card_1",
            active_run_status: "interrupted",
            latest_run_status: "interrupted",
        };
        const refreshBtn = rootEl.querySelector('[data-testid="rl-btn-refresh"]');
        await act(async () => {
            refreshBtn.click();
        });
        await waitForCondition(() => {
            const badge = rootEl.querySelector('[data-run-status="interrupted"]');
            assert.ok(badge, "interrupted status badge must render");
            assert.equal(badge.getAttribute("data-tone"), "unknown");
        });

        await act(async () => {
            root.unmount();
        });
        await cordis.fiber.dispose();
    } finally {
        Object.assign(globalThis, prevGlobals);
        dom.window.close();
        server.closeAllConnections();
        await new Promise((r) => server.close(() => r()));
    }
});
