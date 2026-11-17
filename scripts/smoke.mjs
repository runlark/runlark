#!/usr/bin/env node
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { act } from "react";
import {
    apiJson,
    getFreePort,
    initGitRepo,
    mountUiCard,
    queryDb,
    setDshFault,
    startDaemon,
    startDshHost,
    stopProcess,
    waitFor,
} from "./smoke-harness.mjs";

console.log("Verifying DSH controller patch & building DSH client UI bundle...");
execFileSync("node", ["scripts/apply-dsh-patch.mjs", "--check"], { stdio: "inherit" });
execFileSync("node", ["scripts/build-dsh-client.mjs"], { stdio: "inherit" });
console.log("Compiling runlark binary...");
execFileSync("cargo", ["build", "--bin", "runlark"], { stdio: "inherit" });

const baseDir = mkdtempSync(join(tmpdir(), "runlark-a0-smoke-"));
const repoDir = join(baseDir, "workspace");
const daemonDataDir = join(baseDir, "daemon-data");
const dshStateDir = join(baseDir, "dsh-state");
const dbPath = join(daemonDataDir, "runlark.db");
const binaryPath = join(process.cwd(), "target", "debug", "runlark");

const daemonPort = await getFreePort();
const dshPort = await getFreePort();
let activeDsh = null;
let activeDaemon = null;

try {
    mkdirSync(repoDir, { recursive: true });
    mkdirSync(daemonDataDir, { recursive: true });
    mkdirSync(dshStateDir, { recursive: true });
    const git = initGitRepo(repoDir);

    console.log("Starting separate DSH host and Runlark daemon processes...");
    activeDsh = await startDshHost(`http://127.0.0.1:${daemonPort}`, dshStateDir, dshPort);
    activeDaemon = await startDaemon(binaryPath, daemonDataDir, daemonPort, activeDsh.url);
    const { daemonUrl } = activeDaemon;
    const dshOrigin = activeDsh.origin;
    const sessionId = "sess_a0_e2e";

    // 1. Create managed session + exercise outbox retry after 500 failure + isolated git worktree
    const created = await apiJson(`${dshOrigin}/api/runlark/sessions`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ session_id: sessionId, task_id: "task_a0_1", cwd: repoDir }),
    });
    assert.equal(created.status, 201);
    assert.equal(created.json.current_revision, 1);

    const send1CreatedAt = Date.now();
    setDshFault(dshStateDir, "failOnce", "cmd_a0_send_1");
    const send1 = await apiJson(`${daemonUrl}/api/v1/commands`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
            type: "SendPrompt",
            payload: {
                command_id: "cmd_a0_send_1",
                session_id: sessionId,
                run_id: "run_a0_1",
                prompt: "WAIT_CANCEL initial worktree task",
                expected_revision: 1,
                created_at: send1CreatedAt,
            },
        }),
    });
    assert.equal(send1.status, 201);

    await waitFor(
        () => {
            const rows = queryDb(
                dbPath,
                "SELECT status, error_message FROM commands WHERE command_id = ?",
                "cmd_a0_send_1",
            );
            return rows[0]?.status === "failed" && typeof rows[0]?.error_message === "string";
        },
        4000,
        "outbox failed state",
    );

    await waitFor(
        () => {
            const rows = queryDb(
                dbPath,
                "SELECT status, error_message, dispatch_attempts FROM commands WHERE command_id = ?",
                "cmd_a0_send_1",
            );
            return (
                rows[0]?.status === "acked" &&
                rows[0]?.error_message === null &&
                rows[0]?.dispatch_attempts >= 2
            );
        },
        6000,
        "outbox retry recovery",
    );

    const expectedWt = realpathSync(join(repoDir, ".runlark", "worktrees", "run_a0_1"));
    const sessAfterSend = await apiJson(`${daemonUrl}/api/v1/sessions/${sessionId}`);
    assert.equal(sessAfterSend.json.worktree_path, expectedWt);
    assert.equal(sessAfterSend.json.active_run_status, "running");
    assert.ok(git("worktree", "list", "--porcelain").includes(expectedWt));
    const dshSessFile = JSON.parse(
        readFileSync(join(dshStateDir, "sessions", `${sessionId}.json`), "utf8"),
    );
    assert.equal(realpathSync(dshSessFile.header.cwd), expectedWt);

    const replay = await apiJson(`${daemonUrl}/api/v1/commands`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
            type: "SendPrompt",
            payload: {
                command_id: "cmd_a0_send_1",
                session_id: sessionId,
                run_id: "run_a0_1",
                prompt: "WAIT_CANCEL initial worktree task",
                expected_revision: 1,
                created_at: send1CreatedAt,
            },
        }),
    });
    assert.equal(replay.status, 200);
    const conflictPayload = await apiJson(`${daemonUrl}/api/v1/commands`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
            type: "SendPrompt",
            payload: {
                command_id: "cmd_a0_send_1",
                session_id: sessionId,
                run_id: "run_a0_1",
                prompt: "DIFFERENT PAYLOAD",
                expected_revision: 1,
                created_at: Date.now(),
            },
        }),
    });
    assert.equal(conflictPayload.status, 409);

    // 2. Mount UI card, disconnect browser, verify run continues; Stop -> CancelRequested != Stopped
    const ui1 = mountUiCard(dshOrigin, sessionId, expectedWt);
    const ui1Handle = await ui1.open();
    await waitFor(
        async () => {
            await act(async () => new Promise((r) => setTimeout(r, 30)));
            return Boolean(ui1.rootEl.querySelector('[data-run-status="running"]'));
        },
        4000,
        "UI running status",
    );
    await ui1Handle.close();
    await new Promise((r) => setTimeout(r, 250));
    const afterBrowserClose = await apiJson(`${daemonUrl}/api/v1/sessions/${sessionId}`);
    assert.equal(
        afterBrowserClose.json.active_run_status,
        "running",
        "browser disconnect must not stop run",
    );

    const stopRes = await apiJson(`${dshOrigin}/api/runlark/sessions/${sessionId}/stop`, {
        method: "POST",
    });
    assert.equal(stopRes.status, 200);
    const cancelling = await apiJson(`${daemonUrl}/api/v1/sessions/${sessionId}`);
    assert.equal(
        cancelling.json.active_run_status,
        "cancel_requested",
        "Stop must set CancelRequested, not Stopped",
    );

    await waitFor(
        async () => {
            const s = await apiJson(`${daemonUrl}/api/v1/sessions/${sessionId}`);
            return s.json.active_run_id === null && s.json.latest_run_status === "stopped";
        },
        4000,
        "turn/end settles run_a0_1 to stopped",
    );

    // 3. Separate restart of Rust daemon + separate restart of DSH host + Interrupted + Resume after_seq + 504 Unknown
    const send2 = await apiJson(`${daemonUrl}/api/v1/commands`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
            type: "SendPrompt",
            payload: {
                command_id: "cmd_a0_send_2",
                session_id: sessionId,
                run_id: "run_a0_2",
                prompt: "WAIT_CANCEL restart test",
                expected_revision: 4,
                created_at: Date.now(),
            },
        }),
    });
    assert.equal(send2.status, 201);
    await waitFor(
        async () => {
            const s = await apiJson(`${daemonUrl}/api/v1/sessions/${sessionId}`);
            return s.json.active_run_status === "running";
        },
        4000,
        "run_a0_2 running",
    );

    const runsBeforeRestart = queryDb(
        dbPath,
        "SELECT run_id FROM runs WHERE session_id = ?",
        sessionId,
    );
    console.log("Restarting Rust daemon independently while run_a0_2 is inflight...");
    await stopProcess(activeDaemon, "daemon#1");
    activeDaemon = await startDaemon(binaryPath, daemonDataDir, daemonPort, activeDsh.url);

    const afterDaemonRestart = await apiJson(`${daemonUrl}/api/v1/sessions/${sessionId}`);
    assert.equal(afterDaemonRestart.json.active_run_id, "run_a0_2");
    assert.equal(afterDaemonRestart.json.active_run_status, "interrupted");
    assert.equal(
        queryDb(dbPath, "SELECT run_id FROM runs WHERE session_id = ?", sessionId).length,
        runsBeforeRestart.length,
        "Daemon restart must not create duplicate run",
    );

    console.log("Restarting DSH host independently...");
    await stopProcess(activeDsh, "dsh-host#1");
    activeDsh = await startDshHost(`http://127.0.0.1:${daemonPort}`, dshStateDir, dshPort);
    assert.equal(
        queryDb(dbPath, "SELECT run_id FROM runs WHERE session_id = ?", sessionId).length,
        runsBeforeRestart.length,
        "DSH restart must not create duplicate run",
    );

    const eventsBeforeResume = await apiJson(`${daemonUrl}/api/v1/runs/run_a0_2/events`);
    const lastSeqBeforeResume = eventsBeforeResume.json.at(-1).seq;

    const ui2 = mountUiCard(dshOrigin, sessionId, expectedWt);
    const ui2Handle = await ui2.open();
    await waitFor(
        async () => {
            await act(async () => new Promise((r) => setTimeout(r, 30)));
            return Boolean(
                ui2.rootEl.querySelector('[data-run-status="interrupted"][data-tone="unknown"]'),
            );
        },
        4000,
        "UI shows Interrupted unknown state",
    );

    const resumeRes = await apiJson(`${dshOrigin}/api/runlark/sessions/${sessionId}/resume`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ afterSeq: lastSeqBeforeResume }),
    });
    assert.equal(resumeRes.status, 200);
    assert.equal(resumeRes.json.session.active_run_status, "running");

    const catchupEvents = await apiJson(
        `${daemonUrl}/api/v1/runs/run_a0_2/events?after_seq=${lastSeqBeforeResume}`,
    );
    assert.ok(catchupEvents.json.some((e) => e.event_type === "SessionResumed"));
    assert.ok(catchupEvents.json.every((e) => e.seq > lastSeqBeforeResume));

    setDshFault(dshStateDir, "unknownOnce", "cmd_a0_stop_unk");
    const revBeforeStop = resumeRes.json.session.current_revision;
    const stopUnk = await apiJson(`${daemonUrl}/api/v1/commands`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
            type: "StopRun",
            payload: {
                command_id: "cmd_a0_stop_unk",
                session_id: sessionId,
                run_id: "run_a0_2",
                reason: "unknown_test",
                expected_revision: revBeforeStop,
                created_at: Date.now(),
            },
        }),
    });
    assert.equal(stopUnk.status, 201);
    await waitFor(
        () => {
            const rows = queryDb(
                dbPath,
                "SELECT status FROM commands WHERE command_id = ?",
                "cmd_a0_stop_unk",
            );
            return rows[0]?.status === "dispatched";
        },
        4000,
        "504 unknown keeps dispatched",
    );
    await waitFor(
        async () => {
            await act(async () => new Promise((r) => setTimeout(r, 30)));
            return Boolean(ui2.rootEl.querySelector('[data-tone="unknown"]'));
        },
        4000,
        "UI shows slate unknown delivery badge",
    );
    await ui2Handle.close();

    await apiJson(`${daemonUrl}/api/v1/dsh/events`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
            TurnEnded: {
                session_id: sessionId,
                run_id: "run_a0_2",
                outcome: "Stopped",
            },
        }),
    });
    const reconciledCmd = queryDb(
        dbPath,
        "SELECT status FROM commands WHERE command_id = ?",
        "cmd_a0_stop_unk",
    );
    assert.equal(reconciledCmd[0]?.status, "acked");

    // 4. WaitingForApproval survives BOTH restarts, auto-opens UI, and answering transitions to Running
    const sessPreQ = await apiJson(`${daemonUrl}/api/v1/sessions/${sessionId}`);
    const sendQ = await apiJson(`${daemonUrl}/api/v1/commands`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
            type: "SendPrompt",
            payload: {
                command_id: "cmd_a0_send_q",
                session_id: sessionId,
                run_id: "run_a0_3",
                prompt: "ASK_QUESTION",
                expected_revision: sessPreQ.json.current_revision,
                created_at: Date.now(),
            },
        }),
    });
    assert.equal(sendQ.status, 201);
    await waitFor(
        async () => {
            const s = await apiJson(`${daemonUrl}/api/v1/sessions/${sessionId}`);
            return (
                s.json.active_run_status === "waiting_for_approval" &&
                s.json.pending_question !== null
            );
        },
        5000,
        "waiting_for_approval persisted",
    );

    console.log("Restarting BOTH Rust daemon and DSH host while WaitingForApproval...");
    await stopProcess(activeDaemon, "daemon#2");
    await stopProcess(activeDsh, "dsh-host#2");
    activeDsh = await startDshHost(`http://127.0.0.1:${daemonPort}`, dshStateDir, dshPort);
    activeDaemon = await startDaemon(binaryPath, daemonDataDir, daemonPort, activeDsh.url);

    const ui3 = mountUiCard(dshOrigin, sessionId, expectedWt, true);
    const ui3Handle = await ui3.open();
    await waitFor(
        async () => {
            await act(async () => new Promise((r) => setTimeout(r, 30)));
            return (
                ui3Handle.openedTabs.includes("runlark") &&
                Boolean(ui3.rootEl.querySelector('[data-run-status="waiting_for_approval"]')) &&
                Boolean(ui3.rootEl.querySelector('[data-testid="rl-question-card"]'))
            );
        },
        5000,
        "UI restores WaitingForApproval & auto-opens Runlark tab",
    );

    const form = ui3.rootEl.querySelector('[data-testid="rl-question-card"] form');
    await act(async () => {
        form.dispatchEvent(new ui3.dom.window.Event("submit", { bubbles: true, cancelable: true }));
    });
    await waitFor(
        async () => {
            await act(async () => new Promise((r) => setTimeout(r, 30)));
            return Boolean(ui3.rootEl.querySelector('[data-run-status="running"]'));
        },
        4000,
        "Answering question from UI returns run to Running",
    );
    await ui3Handle.close();

    // Finish run_a0_3 cleanly on restarted DSH host, then ChangeCwd & Archive
    const sessRunning3 = await apiJson(`${daemonUrl}/api/v1/sessions/${sessionId}`);
    const sendFinish = await apiJson(`${daemonUrl}/api/v1/commands`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
            type: "SendPrompt",
            payload: {
                command_id: "cmd_a0_finish_3",
                session_id: sessionId,
                run_id: "run_a0_3",
                prompt: "Finish after approval",
                expected_revision: sessRunning3.json.current_revision,
                created_at: Date.now(),
            },
        }),
    });
    assert.equal(sendFinish.status, 201);
    await waitFor(
        async () => {
            const s = await apiJson(`${daemonUrl}/api/v1/sessions/${sessionId}`);
            return s.json.active_run_id === null && s.json.latest_run_status === "completed";
        },
        5000,
        "run_a0_3 completed after restart+answer",
    );

    const newCwd = join(baseDir, "workspace-next");
    const sessIdle = await apiJson(`${daemonUrl}/api/v1/sessions/${sessionId}`);
    const cwdRes = await apiJson(`${daemonUrl}/api/v1/commands`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
            type: "ChangeCwd",
            payload: {
                command_id: "cmd_a0_cwd",
                session_id: sessionId,
                run_id: "run_a0_3",
                new_cwd: newCwd,
                expected_revision: sessIdle.json.current_revision,
                created_at: Date.now(),
            },
        }),
    });
    assert.equal(cwdRes.status, 201);
    await waitFor(
        () => {
            const rows = queryDb(
                dbPath,
                "SELECT status FROM commands WHERE command_id = ?",
                "cmd_a0_cwd",
            );
            return rows[0]?.status === "acked";
        },
        4000,
        "ChangeCwd acked",
    );

    const archiveRes = await apiJson(`${dshOrigin}/api/runlark/sessions/${sessionId}/archive`, {
        method: "POST",
    });
    assert.equal(archiveRes.status, 200);
    const archivedSess = await apiJson(`${daemonUrl}/api/v1/sessions/${sessionId}`);
    assert.equal(archivedSess.json.status, "archived");
    const archivedList = JSON.parse(readFileSync(join(dshStateDir, "archived.json"), "utf8"));
    assert.ok(archivedList.includes(sessionId));

    console.log(
        "A0 E2E failure-path smoke PASSED: worktree/CWD confinement, 5-op Rust commands + outbox retry/504 unknown, CancelRequested!=Stopped, browser disconnect survival, independent daemon+DSH restarts, after_seq resume, and WaitingForApproval UI restore verified.",
    );
} finally {
    await stopProcess(activeDaemon, "daemon-final").catch(() => {});
    await stopProcess(activeDsh, "dsh-final").catch(() => {});
    rmSync(baseDir, { recursive: true, force: true });
}
