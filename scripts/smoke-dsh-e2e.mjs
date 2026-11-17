#!/usr/bin/env node
import assert from "node:assert/strict";
import { execFileSync, spawn, spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, realpathSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { act } from "react";
import {
    apiJson,
    crashProcess,
    getFreePort,
    initGitRepo,
    mountUiCard,
    queryDb,
    startDaemon,
    stopProcess,
    submitCommand,
    waitFor,
    writeMockLlmPatch,
} from "./smoke-harness.mjs";

function findDshBinary() {
    const res = spawnSync("which", ["dsh"], { encoding: "utf8" });
    return res.status === 0 ? res.stdout.trim() : null;
}

async function startRealDsh(dshBin, dshHome, patchYml, port = 0) {
    const proc = spawn(
        dshBin,
        ["--profile", "runlark", "--patch", patchYml, "--no-open", "--port", String(port)],
        {
            env: { ...process.env, DSH_HOME: dshHome },
            stdio: ["ignore", "pipe", "pipe"],
        },
    );
    let stdout = "";
    let stderr = "";
    proc.stdout.on("data", (d) => {
        stdout += d.toString();
    });
    proc.stderr.on("data", (d) => {
        stderr += d.toString();
    });

    const url = await waitFor(
        () => {
            const m = /https?:\/\/127\.0\.0\.1:\d+\/\?token=[^\s]+/.exec(`${stdout}\n${stderr}`);
            return m ? m[0] : false;
        },
        15_000,
        "real dsh web URL",
    ).catch((err) => {
        proc.kill("SIGKILL");
        throw new Error(`Real dsh failed to start: ${err.message}\nOUT:${stdout}\nERR:${stderr}`);
    });

    return {
        proc,
        url,
        origin: url.split("/?")[0],
        getLogs: () => ({ stdout, stderr }),
    };
}

const dshBin = findDshBinary();
if (!dshBin) {
    console.log("Real dsh binary not found on PATH; skipping real-dsh-binary local E2E check.");
    process.exit(0);
}

execFileSync("node", ["scripts/apply-dsh-patch.mjs", "--check"], { stdio: "inherit" });
execFileSync("node", ["scripts/build-dsh-client.mjs"], { stdio: "inherit" });
execFileSync("cargo", ["build", "--bin", "runlark"], { stdio: "inherit" });

const baseDir = mkdtempSync(join(tmpdir(), "runlark-real-dsh-e2e-"));
const dshHome = join(baseDir, "dsh-home");
const repoDir = join(baseDir, "workspace");
const daemonDataDir = join(baseDir, "daemon-data");
const dbPath = join(daemonDataDir, "runlark.db");
const binaryPath = join(process.cwd(), "target", "debug", "runlark");
const daemonPort = await getFreePort();
const dshPort = await getFreePort();
const daemonUrl = `http://127.0.0.1:${daemonPort}`;

let activeDaemon = null;
let activeDsh = null;

try {
    mkdirSync(dshHome, { recursive: true });
    mkdirSync(repoDir, { recursive: true });
    mkdirSync(daemonDataDir, { recursive: true });
    const git = initGitRepo(repoDir);

    execFileSync("node", ["scripts/setup-dsh-profile.mjs", "--daemon-url", daemonUrl], {
        env: { ...process.env, DSH_HOME: dshHome },
        stdio: "inherit",
    });

    const patchYml = writeMockLlmPatch(dshHome);

    console.log("Booting real dsh-v0.2.2-rc.3 binary + Runlark daemon...");
    activeDsh = await startRealDsh(dshBin, dshHome, patchYml, dshPort);
    activeDaemon = await startDaemon(binaryPath, daemonDataDir, daemonPort, activeDsh.url);

    const health = await apiJson(`${activeDsh.origin}/api/runlark/health`);
    assert.equal(health.status, 200);
    assert.deepEqual(health.json, { status: "ok", daemonHealthy: true });

    const sessionId = "sess_real_dsh_1";
    const created = await apiJson(`${activeDsh.origin}/api/runlark/sessions`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ session_id: sessionId, task_id: "task_real_1", cwd: repoDir }),
    });
    assert.equal(created.status, 201);
    assert.equal(created.json.current_revision, 1);

    // 1. Outbox failure -> retry recovery (error_message = NULL), isolated git worktree + CWD confinement,
    //    idempotent replay + 409 conflict, UI close survival, Stop -> CancelRequested != Stopped
    await stopProcess(activeDsh, "real-dsh#pre-retry");
    activeDsh = null;

    const send1CreatedAt = Date.now();
    const send1 = await submitCommand(daemonUrl, "SendPrompt", {
        command_id: "cmd_real_send_1",
        session_id: sessionId,
        run_id: "run_real_1",
        prompt: "WAIT_CANCEL real dsh test",
        expected_revision: 1,
        created_at: send1CreatedAt,
    });
    assert.equal(send1.status, 201);

    await waitFor(
        () => {
            const rows = queryDb(
                dbPath,
                "SELECT status, error_message FROM commands WHERE command_id = ?",
                "cmd_real_send_1",
            );
            return rows[0]?.status === "failed" && typeof rows[0]?.error_message === "string";
        },
        5000,
        "real dsh outbox failed state while dsh offline",
    );

    activeDsh = await startRealDsh(dshBin, dshHome, patchYml, dshPort);
    await waitFor(
        () => {
            const rows = queryDb(
                dbPath,
                "SELECT status, error_message, dispatch_attempts FROM commands WHERE command_id = ?",
                "cmd_real_send_1",
            );
            return (
                rows[0]?.status === "acked" &&
                rows[0]?.error_message === null &&
                rows[0]?.dispatch_attempts >= 2
            );
        },
        10_000,
        "real dsh outbox retry recovery",
    );

    const replay = await submitCommand(daemonUrl, "SendPrompt", {
        command_id: "cmd_real_send_1",
        session_id: sessionId,
        run_id: "run_real_1",
        prompt: "WAIT_CANCEL real dsh test",
        expected_revision: 1,
        created_at: send1CreatedAt,
    });
    assert.equal(replay.status, 200);
    const conflict = await submitCommand(daemonUrl, "SendPrompt", {
        command_id: "cmd_real_send_1",
        session_id: sessionId,
        run_id: "run_real_1",
        prompt: "CONFLICTING_PAYLOAD",
        expected_revision: 1,
        created_at: Date.now(),
    });
    assert.equal(conflict.status, 409);

    const expectedWt = realpathSync(join(repoDir, ".runlark", "worktrees", "run_real_1"));
    assert.ok(git("worktree", "list", "--porcelain").includes(expectedWt));
    const sess1 = await apiJson(`${daemonUrl}/api/v1/sessions/${sessionId}`);
    assert.equal(sess1.json.worktree_path, expectedWt);
    await waitFor(
        () =>
            queryDb(dbPath, "SELECT event_type FROM events WHERE run_id = ?", "run_real_1").some(
                (r) => r.event_type === "DshTurnStarted",
            ),
        8000,
        "real dsh turn/start recorded",
    );
    await waitFor(
        () =>
            queryDb(
                dbPath,
                "SELECT payload FROM events WHERE run_id = ? AND event_type = 'DshUserMessage'",
                "run_real_1",
            ).some((r) => r.payload.includes(expectedWt)),
        8000,
        "real dsh runtime-context confined to worktree CWD",
    );

    // Browser/UI client disconnect must not interrupt active execution
    const ui1 = mountUiCard(activeDsh.origin, sessionId, expectedWt);
    const ui1Handle = await ui1.open();
    await waitFor(
        async () => {
            await act(async () => new Promise((r) => setTimeout(r, 30)));
            return Boolean(ui1.rootEl.querySelector('[data-run-status="running"]'));
        },
        4000,
        "real dsh UI running status",
    );
    await ui1Handle.close();
    await new Promise((r) => setTimeout(r, 250));
    const afterUiClose = await apiJson(`${daemonUrl}/api/v1/sessions/${sessionId}`);
    assert.equal(afterUiClose.json.active_run_status, "running");

    const stopRes = await apiJson(`${activeDsh.origin}/api/runlark/sessions/${sessionId}/stop`, {
        method: "POST",
    });
    assert.equal(stopRes.status, 200);
    const cancelling = await apiJson(`${daemonUrl}/api/v1/sessions/${sessionId}`);
    assert.equal(cancelling.json.active_run_status, "cancel_requested");

    await waitFor(
        async () => {
            const s = await apiJson(`${daemonUrl}/api/v1/sessions/${sessionId}`);
            return s.json.active_run_id === null && s.json.latest_run_status === "stopped";
        },
        8000,
        "real dsh turn/end settles run_real_1 to stopped",
    );

    // 2. Separate restart of Rust daemon + separate restart of real DSH -> Interrupted -> Resume after_seq -> Complete
    const preRun2 = await apiJson(`${daemonUrl}/api/v1/sessions/${sessionId}`);
    const send2 = await submitCommand(daemonUrl, "SendPrompt", {
        command_id: "cmd_real_send_2",
        session_id: sessionId,
        run_id: "run_real_2",
        prompt: "WAIT_CANCEL real dsh restart test",
        expected_revision: preRun2.json.current_revision,
        created_at: Date.now(),
    });
    assert.equal(send2.status, 201);
    await waitFor(
        () =>
            queryDb(dbPath, "SELECT event_type FROM events WHERE run_id = ?", "run_real_2").some(
                (r) => r.event_type === "DshTurnStarted",
            ),
        8000,
        "run_real_2 turn/start recorded",
    );

    const runsBeforeRestart = queryDb(
        dbPath,
        "SELECT run_id FROM runs WHERE session_id = ?",
        sessionId,
    );
    console.log("Restarting Rust daemon independently while real dsh run_real_2 is inflight...");
    await stopProcess(activeDaemon, "real-dsh-daemon#1");
    activeDaemon = await startDaemon(binaryPath, daemonDataDir, daemonPort, activeDsh.url);

    const afterDaemonRestart = await apiJson(`${daemonUrl}/api/v1/sessions/${sessionId}`);
    assert.equal(afterDaemonRestart.json.active_run_id, "run_real_2");
    assert.equal(afterDaemonRestart.json.active_run_status, "interrupted");
    assert.equal(
        queryDb(dbPath, "SELECT run_id FROM runs WHERE session_id = ?", sessionId).length,
        runsBeforeRestart.length,
    );

    console.log("Restarting real dsh independently...");
    await crashProcess(activeDsh, "real-dsh#1");
    activeDsh = await startRealDsh(dshBin, dshHome, patchYml, dshPort);
    assert.equal(
        queryDb(dbPath, "SELECT run_id FROM runs WHERE session_id = ?", sessionId).length,
        runsBeforeRestart.length,
    );

    const eventsBeforeResume = await apiJson(`${daemonUrl}/api/v1/runs/run_real_2/events`);
    const lastSeqBeforeResume = eventsBeforeResume.json.at(-1).seq;

    const ui2 = mountUiCard(activeDsh.origin, sessionId, expectedWt);
    const ui2Handle = await ui2.open();
    await waitFor(
        async () => {
            await act(async () => new Promise((r) => setTimeout(r, 30)));
            return Boolean(
                ui2.rootEl.querySelector('[data-run-status="interrupted"][data-tone="unknown"]'),
            );
        },
        4000,
        "real dsh UI shows Interrupted unknown state",
    );
    await ui2Handle.close();

    const resumeRes = await apiJson(
        `${activeDsh.origin}/api/runlark/sessions/${sessionId}/resume`,
        {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ afterSeq: lastSeqBeforeResume }),
        },
    );
    assert.equal(resumeRes.status, 200);
    assert.equal(resumeRes.json.session.active_run_status, "running");

    const catchupEvents = await apiJson(
        `${daemonUrl}/api/v1/runs/run_real_2/events?after_seq=${lastSeqBeforeResume}`,
    );
    assert.ok(catchupEvents.json.some((e) => e.event_type === "SessionResumed"));
    assert.ok(catchupEvents.json.every((e) => e.seq > lastSeqBeforeResume));

    const finishResumed = await submitCommand(daemonUrl, "SendPrompt", {
        command_id: "cmd_real_finish_2",
        session_id: sessionId,
        run_id: "run_real_2",
        prompt: "Finish resumed run",
        expected_revision: resumeRes.json.session.current_revision,
        created_at: Date.now(),
    });
    assert.equal(finishResumed.status, 201);
    await waitFor(
        async () => {
            const s = await apiJson(`${daemonUrl}/api/v1/sessions/${sessionId}`);
            return s.json.active_run_id === null && s.json.latest_run_status === "completed";
        },
        8000,
        "resumed run_real_2 completes on real dsh",
    );

    // 3. WaitingForApproval persists in SQLite across BOTH restarts, restores in DSH UI card, and completes on answer
    const preQ = await apiJson(`${daemonUrl}/api/v1/sessions/${sessionId}`);
    const sendQ = await submitCommand(daemonUrl, "SendPrompt", {
        command_id: "cmd_real_send_q",
        session_id: sessionId,
        run_id: "run_real_3",
        prompt: "ASK_QUESTION",
        expected_revision: preQ.json.current_revision,
        created_at: Date.now(),
    });
    assert.equal(sendQ.status, 201);

    await waitFor(
        async () => {
            const s = await apiJson(`${daemonUrl}/api/v1/sessions/${sessionId}`);
            return s.json.active_run_status === "waiting_for_approval" && s.json.pending_question
                ? s.json
                : false;
        },
        6000,
        "real dsh WaitingForApproval",
    );

    console.log("Restarting BOTH Rust daemon and real dsh while WaitingForApproval...");
    await stopProcess(activeDaemon, "real-dsh-daemon#2");
    await crashProcess(activeDsh, "real-dsh#2");
    activeDsh = await startRealDsh(dshBin, dshHome, patchYml, dshPort);
    activeDaemon = await startDaemon(binaryPath, daemonDataDir, daemonPort, activeDsh.url);

    const qRows = queryDb(
        dbPath,
        "SELECT status FROM questions WHERE question_id = ?",
        "q_real_dsh_1",
    );
    assert.equal(qRows[0]?.status, "pending");

    const ui3 = mountUiCard(activeDsh.origin, sessionId, expectedWt, true);
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
        6000,
        "real dsh UI restores WaitingForApproval & auto-opens Runlark tab",
    ).catch(async (err) => {
        const s = await apiJson(`${activeDsh.origin}/api/runlark/sessions/${sessionId}`);
        console.error("DEBUG ui3 openedTabs:", ui3Handle.openedTabs);
        console.error("DEBUG ui3 proxy session:", JSON.stringify(s));
        console.error("DEBUG ui3 html:", ui3.rootEl.innerHTML.slice(0, 600));
        throw err;
    });

    const form = ui3.rootEl.querySelector('[data-testid="rl-question-card"] form');
    await act(async () => {
        form.dispatchEvent(new ui3.dom.window.Event("submit", { bubbles: true, cancelable: true }));
    });
    await waitFor(
        async () => {
            await act(async () => new Promise((r) => setTimeout(r, 30)));
            return Boolean(
                ui3.rootEl.querySelector('[data-run-status="running"]') ||
                ui3.rootEl.querySelector('[data-run-status="completed"]'),
            );
        },
        4000,
        "Answering question from UI returns real dsh run to Running/Completed",
    ).catch(async (err) => {
        const s = await apiJson(`${daemonUrl}/api/v1/sessions/${sessionId}`);
        const evs = await apiJson(`${daemonUrl}/api/v1/runs/run_real_3/events`);
        console.error("DEBUG post-submit session:", JSON.stringify(s.json));
        console.error("DEBUG post-submit events:", JSON.stringify(evs.json));
        console.error("DEBUG post-submit ui3 html:", ui3.rootEl.innerHTML.slice(0, 800));
        throw err;
    });
    await ui3Handle.close();

    await waitFor(
        async () => {
            const s = await apiJson(`${daemonUrl}/api/v1/sessions/${sessionId}`);
            return s.json.active_run_id === null && s.json.latest_run_status === "completed";
        },
        8000,
        "real dsh run_real_3 completed after restart+answer",
    );

    // 4. ChangeCwd + ArchiveSession
    const newCwd = join(baseDir, "workspace-next");
    const preCwd = await apiJson(`${daemonUrl}/api/v1/sessions/${sessionId}`);
    const cwdRes = await submitCommand(daemonUrl, "ChangeCwd", {
        command_id: "cmd_real_cwd",
        session_id: sessionId,
        run_id: "run_real_3",
        new_cwd: newCwd,
        expected_revision: preCwd.json.current_revision,
        created_at: Date.now(),
    });
    assert.equal(cwdRes.status, 201);
    await waitFor(
        () => {
            const rows = queryDb(
                dbPath,
                "SELECT status FROM commands WHERE command_id = ?",
                "cmd_real_cwd",
            );
            return rows[0]?.status === "acked";
        },
        5000,
        "real dsh ChangeCwd acked",
    );
    const afterCwd = await apiJson(`${daemonUrl}/api/v1/sessions/${sessionId}`);
    assert.equal(afterCwd.json.cwd, newCwd);

    const archiveRes = await apiJson(
        `${activeDsh.origin}/api/runlark/sessions/${sessionId}/archive`,
        {
            method: "POST",
        },
    );
    assert.equal(archiveRes.status, 200);
    const archived = await apiJson(`${daemonUrl}/api/v1/sessions/${sessionId}`);
    assert.equal(archived.json.status, "archived");

    console.log(
        "Real dsh-v0.2.2-rc.3 binary E2E smoke PASSED (worktree/CWD confinement, 5-op Rust commands + outbox retry/409, CancelRequested!=Stopped, UI disconnect survival, separate daemon+DSH restarts, after_seq resume, WaitingForApproval UI restore->answer->Completed, Archive).",
    );
} finally {
    await stopProcess(activeDaemon, "real-dsh-daemon").catch(() => {});
    await stopProcess(activeDsh, "real-dsh").catch(() => {});
    rmSync(baseDir, { recursive: true, force: true });
}
