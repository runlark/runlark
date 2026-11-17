import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createRequire } from "node:module";
import { JSDOM } from "jsdom";
import React, { act } from "react";
import ReactDom from "react-dom";
import * as ReactDomClient from "react-dom/client";
import { createRoot } from "react-dom/client";
import * as ReactJsxRuntime from "react/jsx-runtime";
import {
    apiJson,
    getFreePort,
    initGitRepo,
    startDaemon,
    startDshHost,
    stopProcess,
    waitFor,
} from "./smoke-harness.mjs";

const require = createRequire(import.meta.url);

export async function runA1SoloAlphaSmoke(binaryPath) {
    console.log(
        "Running A1 solo-alpha E2E smoke (2-slot scheduler, repair loop, bound acceptance, board UI)...",
    );
    const baseDir = mkdtempSync(join(tmpdir(), "runlark-a1-smoke-"));
    const repoDir = join(baseDir, "repo");
    const daemonDataDir = join(baseDir, "daemon-data");
    const dshStateDir = join(baseDir, "dsh-state");
    const daemonPort = await getFreePort();
    const dshPort = await getFreePort();

    let activeDsh = null;
    let activeDaemon = null;

    try {
        mkdirSync(repoDir, { recursive: true });
        mkdirSync(daemonDataDir, { recursive: true });
        mkdirSync(dshStateDir, { recursive: true });
        initGitRepo(repoDir);

        activeDsh = await startDshHost(`http://127.0.0.1:${daemonPort}`, dshStateDir, dshPort);
        activeDaemon = await startDaemon(binaryPath, daemonDataDir, daemonPort, activeDsh.url);
        const { daemonUrl } = activeDaemon;
        const dshOrigin = activeDsh.origin;

        // 1. Register project via DSH proxy + verify CLI `task board` works
        const proj = await apiJson(`${dshOrigin}/api/runlark/projects`, {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({
                project_id: "proj_a1_smoke",
                name: "A1 Smoke Repo",
                repo_root: repoDir,
                default_branch: "main",
            }),
        });
        assert.equal(proj.status, 201);

        // 2. Create 3 tasks via chat commands + API: 2 parallel tasks occupy 2 slots, 3rd queues
        const t1 = await apiJson(`${dshOrigin}/api/runlark/chat`, {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({
                project_id: "proj_a1_smoke",
                input: "/fix First parallel quickfix",
            }),
        });
        assert.equal(t1.status, 200);
        const task1Id = t1.json.data.id;

        const t2 = await apiJson(`${dshOrigin}/api/runlark/chat`, {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({
                project_id: "proj_a1_smoke",
                input: "/task Second parallel standard task",
            }),
        });
        assert.equal(t2.status, 200);
        const task2Id = t2.json.data.id;

        const t3 = await apiJson(`${dshOrigin}/api/runlark/chat`, {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({
                project_id: "proj_a1_smoke",
                input: "/fix Third queued task",
            }),
        });
        assert.equal(t3.status, 200);
        const task3Id = t3.json.data.id;

        // Advance task 2 through Clarification -> Plan -> Implementation
        const clar = await apiJson(`${dshOrigin}/api/runlark/tasks/${task2Id}/clarify`, {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ clarification: "Scope confirmed" }),
        });
        assert.equal(clar.status, 200);
        const plan = await apiJson(`${dshOrigin}/api/runlark/tasks/${task2Id}/plan`, {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ plan_text: "Implement and test", approve: true }),
        });
        assert.equal(plan.status, 200);
        assert.equal(plan.json.workflow_stage, "implementation");

        const s1 = await apiJson(`${dshOrigin}/api/runlark/tasks/${task1Id}/start`, {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ prompt: "WAIT_CANCEL parallel 1" }),
        });
        assert.equal(s1.status, 201);
        assert.equal(s1.json.admitted, true);

        const s2 = await apiJson(`${dshOrigin}/api/runlark/tasks/${task2Id}/start`, {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ prompt: "WAIT_CANCEL parallel 2" }),
        });
        assert.equal(s2.status, 201);
        assert.equal(s2.json.admitted, true);
        assert.equal(s2.json.task.workflow_stage, "implementation");
        assert.notEqual(s1.json.run.worktree_path, s2.json.run.worktree_path);

        const s3 = await apiJson(`${dshOrigin}/api/runlark/tasks/${task3Id}/start`, {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ prompt: "Queued work" }),
        });
        assert.equal(s3.status, 201);
        assert.equal(s3.json.admitted, false);
        assert.equal(s3.json.run_status, "queued");

        // Edit worktree of task 1 and advance to Acceptance via DSH turn/end -> verify_and_advance,
        // freeing slot 1 so task 3 auto-admits. Also verify manual POST /verify on task 2.
        writeFileSync(join(s1.json.run.worktree_path, "fix1.txt"), "fixed\n", "utf8");
        const te1 = await apiJson(`${daemonUrl}/api/v1/dsh/events`, {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({
                TurnEnded: {
                    session_id: s1.json.run.session_id,
                    run_id: s1.json.run.id,
                    outcome: "Completed",
                },
            }),
        });
        assert.equal(te1.status, 200);
        assert.equal(te1.json.run_status, "waiting_for_approval");

        writeFileSync(join(s2.json.run.worktree_path, "fix2.txt"), "fixed standard\n", "utf8");
        const ver2 = await apiJson(`${dshOrigin}/api/runlark/runs/${s2.json.run.id}/verify`, {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ task_id: task2Id }),
        });
        assert.equal(ver2.status, 200);
        assert.equal(ver2.json.verdict, "pass");

        await waitFor(
            async () => {
                const r3 = await apiJson(`${daemonUrl}/api/v1/runs/${s3.json.run.id}`);
                return r3.ok && r3.json.status === "running" && Boolean(r3.json.worktree_path);
            },
            5000,
            "task 3 auto-admitted after task 1 entered WaitingForApproval",
        );

        // Bound human acceptance moves task 1 to Done
        const acc1 = await apiJson(`${dshOrigin}/api/runlark/tasks/${task1Id}/accept`, {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ comment: "Verified A1 smoke" }),
        });
        assert.equal(acc1.status, 200);
        assert.equal(acc1.json.task.stage, "done");

        // Verify RunlarkBoardTabBody renders real daemon board in JSDOM
        const boardSnapshot = await apiJson(`${dshOrigin}/api/runlark/board`);
        assert.equal(boardSnapshot.status, 200);
        assert.equal(boardSnapshot.json.tasks.length, 3);

        // Verify CLI `task board --json` succeeds against live daemon
        const cliBoard = execFileSync(
            binaryPath,
            ["--daemon-url", daemonUrl, "--json", "task", "board"],
            { encoding: "utf8" },
        );
        assert.equal(JSON.parse(cliBoard).tasks.length, 3);

        // Restart daemon independently and ensure board & queued tasks survive cleanly
        await stopProcess(activeDaemon, "a1-daemon-restart");
        activeDaemon = await startDaemon(binaryPath, daemonDataDir, daemonPort, activeDsh.url);
        const afterRestart = await apiJson(`${daemonUrl}/api/v1/board`);
        assert.equal(afterRestart.status, 200);
        assert.equal(afterRestart.json.tasks.length, 3);

        // Mount compiled RunlarkBoardTabBody against live DSH proxy
        const dom = new JSDOM(
            `<!DOCTYPE html><html lang="en"><body><div id="root"></div></body></html>`,
            {
                url: `${dshOrigin}/`,
                runScripts: "dangerously",
            },
        );
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
        dom.window.matchMedia = () => ({
            matches: false,
            media: "",
            addEventListener: () => {},
            removeEventListener: () => {},
        });
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
                    typeof input === "string" && input.startsWith("/")
                        ? `${dshOrigin}${input}`
                        : input;
                return prev.fetch(resolved, init);
            },
        });
        dom.window.fetch = globalThis.fetch;
        dom.window.AbortController = globalThis.AbortController;
        dom.window.AbortSignal = globalThis.AbortSignal;

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
        dom.window.__ModuleLoader__ = {
            load(entry) {
                loaded.set(
                    entry.id,
                    entry.factory((s) => staticModules[s]()),
                );
            },
        };
        const bundlePath = join(process.cwd(), "packages", "dsh-plugin", "dist", "client.js");
        assert.ok(existsSync(bundlePath));
        dom.window.eval(readFileSync(bundlePath, "utf8"));
        const uiPlugin = loaded.get("@runlark/dsh-plugin");
        const rootEl = dom.window.document.getElementById("root");
        const root = createRoot(rootEl);

        try {
            await act(async () => {
                root.render(React.createElement(uiPlugin.RunlarkBoardTabBody));
            });
            await waitFor(
                async () => {
                    await act(async () => new Promise((r) => setTimeout(r, 40)));
                    return (
                        rootEl.textContent.includes("First parallel quickfix") &&
                        rootEl.textContent.includes("Third queued task")
                    );
                },
                5000,
                "A1 live board UI renders all tasks",
            );
        } finally {
            await act(async () => root.unmount());
            dom.window.close();
            Object.assign(globalThis, prev);
        }

        console.log("A1 solo-alpha E2E smoke PASSED.");
    } finally {
        await stopProcess(activeDaemon, "a1-daemon-final").catch(() => {});
        await stopProcess(activeDsh, "a1-dsh-final").catch(() => {});
        rmSync(baseDir, { recursive: true, force: true });
    }
}
