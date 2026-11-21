import assert from "node:assert/strict";
import { execFileSync, spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { join } from "node:path";
import { JSDOM } from "jsdom";
import React, { act } from "react";
import ReactDom from "react-dom";
import * as ReactDomClient from "react-dom/client";
import { createRoot } from "react-dom/client";
import * as ReactJsxRuntime from "react/jsx-runtime";
import { apiJson, stopProcess, waitFor } from "./smoke-harness.mjs";

const require = createRequire(import.meta.url);

export const jsonPost = (body) => ({
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
});

export function bootstrapTeamOwner(serverBin, postgresUrl, opts) {
    const out = execFileSync(
        serverBin,
        [
            "bootstrap-owner",
            "--postgres-url",
            postgresUrl,
            "--user-id",
            opts.userId,
            "--display-name",
            opts.displayName,
            "--workspace-id",
            opts.workspaceId,
            "--workspace-name",
            opts.workspaceName,
            "--workspace-slug",
            opts.workspaceSlug,
        ],
        { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] },
    );
    return JSON.parse(out.trim());
}

export async function startTeamServer(
    serverBin,
    postgresUrl,
    casRoot,
    port,
    heartbeatTimeoutMs = 1200,
) {
    const serverUrl = `http://127.0.0.1:${port}`;
    const proc = spawn(
        serverBin,
        [
            "serve",
            "--host",
            "127.0.0.1",
            "--port",
            String(port),
            "--postgres-url",
            postgresUrl,
            "--cas-root",
            casRoot,
            "--runner-heartbeat-timeout-ms",
            String(heartbeatTimeoutMs),
        ],
        { stdio: ["ignore", "pipe", "pipe"] },
    );
    let stdout = "";
    let stderr = "";
    proc.stdout.on("data", (d) => {
        stdout += d.toString();
    });
    proc.stderr.on("data", (d) => {
        stderr += d.toString();
    });

    await waitFor(
        async () => {
            const res = await fetch(`${serverUrl}/health`, { signal: AbortSignal.timeout(1000) });
            if (!res.ok) return false;
            const json = await res.json();
            return json.status === "ok";
        },
        10_000,
        "runlark-server health",
    ).catch((err) => {
        proc.kill("SIGKILL");
        throw new Error(
            `runlark-server failed to start: ${err.message}\nOUT:${stdout}\nERR:${stderr}`,
        );
    });

    return { proc, serverUrl, getLogs: () => ({ stdout, stderr }) };
}

export async function verifyMountedTeamUi(dshOrigin, expectedTaskTitle) {
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
                typeof input === "string" && input.startsWith("/") ? `${dshOrigin}${input}` : input;
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
    assert.ok(existsSync(bundlePath), "DSH client bundle must exist");
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
                    Boolean(rootEl.querySelector('[data-testid="rl-team-sync-strip"]')) &&
                    rootEl.textContent.includes(expectedTaskTitle)
                );
            },
            6000,
            "A2 live Team UI renders sync strip and promoted task",
        );
        await act(async () => {
            rootEl.querySelector('[data-testid="rl-nav-team"]').click();
        });
        await waitFor(
            async () => {
                await act(async () => new Promise((r) => setTimeout(r, 40)));
                return (
                    Boolean(rootEl.querySelector('[data-testid="rl-team-workspace-view"]')) &&
                    rootEl.textContent.includes("Alice Smoke") &&
                    rootEl.textContent.includes("Bob Smoke")
                );
            },
            6000,
            "A2 live Team view renders synced workspace members",
        );
    } finally {
        await act(async () => root.unmount());
        dom.window.close();
        Object.assign(globalThis, prev);
    }
}

export async function verifyOfflineConflictAndCas(ctx) {
    const {
        dshOrigin,
        d2Url,
        wsId,
        projId,
        taskId,
        teamRunId,
        suffix,
        ownerToken,
        serverBin,
        postgresUrl,
        casRoot,
        serverPort,
    } = ctx;

    // 5. 409 revision conflict, server outage offline gate (503), offline drafts & reconnect replay
    const taskBeforeMove = await apiJson(`${dshOrigin}/api/runlark/tasks/${taskId}`);
    assert.equal(taskBeforeMove.status, 200);
    const revN = taskBeforeMove.json.task.revision;
    const moveAlice = await apiJson(
        `${dshOrigin}/api/runlark/tasks/${taskId}/move`,
        jsonPost({ stage: "ready", actor: "human", expected_revision: revN }),
    );
    assert.equal(moveAlice.status, 200, `moveAlice failed: ${moveAlice.text}`);
    const revNext = moveAlice.json.revision;
    assert.equal(revNext, revN + 1);

    // Bob submits move with stale expected_revision = revN -> 409 with actual_revision
    const staleMoveBob = await apiJson(
        `${d2Url}/api/v1/tasks/${taskId}/move`,
        jsonPost({ stage: "done", actor: "human", expected_revision: revN }),
    );
    assert.equal(staleMoveBob.status, 409, `expected 409 conflict: ${staleMoveBob.text}`);
    assert.equal(staleMoveBob.json.actual_revision, revNext);

    // Stop runlark-server to simulate outage
    await stopProcess(ctx.teamSrv, "a2-team-server-outage");
    ctx.teamSrv = null;

    // Offline gate: authoritative move MUST fail closed with 503 team_server_offline
    const offlineMove = await apiJson(
        `${dshOrigin}/api/runlark/tasks/${taskId}/move`,
        jsonPost({ stage: "in_progress", actor: "human", expected_revision: revNext }),
    );
    assert.equal(offlineMove.status, 503, `expected 503 offline gate: ${offlineMove.text}`);
    assert.equal(offlineMove.json.error, "team_server_offline");

    // Offline comment queues explicitly as 202 OfflineDraft (pending_sync)
    const offlineCmt = await apiJson(
        `${dshOrigin}/api/runlark/tasks/${taskId}/comments`,
        jsonPost({ body: "Offline comment queued during outage" }),
    );
    assert.equal(offlineCmt.status, 202, `expected 202 offline comment: ${offlineCmt.text}`);
    assert.equal(offlineCmt.json.delivery, "queued_offline");
    assert.equal(offlineCmt.json.draft.sync_state.state, "pending_sync");
    const cmtDraftId = offlineCmt.json.draft.id;

    // Offline plan_edit draft with intentionally stale base_revision=1 triggers 409 on reconnect
    const stalePlanDraft = await apiJson(
        `${dshOrigin}/api/runlark/team/drafts`,
        jsonPost({
            project_id: projId,
            task_id: taskId,
            kind: "plan_edit",
            payload_json: JSON.stringify({ plan_text: "Offline plan edit to rebase" }),
            base_revision: 1,
            queue_sync: true,
        }),
    );
    assert.equal(stalePlanDraft.status, 201, `create draft failed: ${stalePlanDraft.text}`);
    const planDraftId = stalePlanDraft.json.id;

    // Restart runlark-server on SAME port, Postgres URL, and CAS root
    ctx.teamSrv = await startTeamServer(serverBin, postgresUrl, casRoot, serverPort, 1200);
    const { serverUrl } = ctx.teamSrv;

    // Reconnect & sync Daemon #1: comment draft replays cleanly; stale plan_edit marks conflict
    const reconnectSync = await apiJson(
        `${dshOrigin}/api/runlark/team/sync`,
        jsonPost({ workspace_id: wsId }),
    );
    assert.equal(reconnectSync.status, 200, `reconnect sync failed: ${reconnectSync.text}`);
    const draftsAfter = await apiJson(`${dshOrigin}/api/runlark/team/drafts?project_id=${projId}`);
    assert.equal(draftsAfter.status, 200);
    const replayedCmt = draftsAfter.json.drafts.find((d) => d.id === cmtDraftId);
    assert.equal(
        replayedCmt?.sync_state.state,
        "synced",
        "offline comment must replay on reconnect",
    );
    const conflictedPlan = draftsAfter.json.drafts.find((d) => d.id === planDraftId);
    assert.equal(
        conflictedPlan?.sync_state.state,
        "conflict",
        "stale plan_edit draft must surface explicit 409 conflict",
    );

    // Retry conflicted draft with current server revision -> synced!
    const retriedPlan = await apiJson(
        `${dshOrigin}/api/runlark/team/drafts/${planDraftId}/retry`,
        jsonPost({ expected_revision: revNext }),
    );
    assert.equal(retriedPlan.status, 200, `retry draft failed: ${retriedPlan.text}`);
    assert.equal(retriedPlan.json.sync_state.state, "synced");

    // Bob syncs and observes the replayed offline comment
    await apiJson(`${d2Url}/api/v1/team/sync`, jsonPost({ workspace_id: wsId }));
    const bobComments2 = await apiJson(`${d2Url}/api/v1/tasks/${taskId}/comments`);
    assert.ok(
        bobComments2.json.some((c) => c.body === "Offline comment queued during outage"),
        "Bob must receive Alice's replayed offline comment",
    );

    // 6. Verified CAS artifact upload -> sync download -> SHA-256 verified fetch through DSH proxy
    const casPayload = JSON.stringify({ verdict: "pass", smoke: "a2" });
    const casDigest = createHash("sha256").update(casPayload).digest("hex");
    const casPut = await fetch(
        `${serverUrl}/api/v1/workspaces/${wsId}/artifacts/${casDigest}?kind=evidence_bundle&media_type=application/json`,
        {
            method: "PUT",
            headers: { Authorization: `Bearer ${ownerToken}` },
            body: casPayload,
        },
    );
    assert.equal(casPut.status, 201, `CAS PUT failed: ${casPut.status}`);
    const attachRes = await apiJson(`${serverUrl}/api/v1/workspaces/${wsId}/commands`, {
        method: "POST",
        headers: {
            "Content-Type": "application/json",
            Authorization: `Bearer ${ownerToken}`,
        },
        body: JSON.stringify({
            type: "AttachArtifact",
            payload: {
                command_id: `cmd_cas_${suffix}`,
                run_id: teamRunId,
                digest_sha256: casDigest,
                kind: "evidence_bundle",
                media_type: "application/json",
                size_bytes: Buffer.byteLength(casPayload),
                label: "evidence_bundle",
                created_at: Date.now(),
            },
        }),
    });
    assert.equal(attachRes.status, 201, `AttachArtifact failed: ${attachRes.text}`);
    await apiJson(`${dshOrigin}/api/runlark/team/sync`, jsonPost({ workspace_id: wsId }));
    const verifiedArt = await fetch(`${dshOrigin}/api/runlark/artifacts/${casDigest}`);
    assert.equal(verifiedArt.status, 200, `Verified CAS proxy fetch failed: ${verifiedArt.status}`);
    assert.equal(verifiedArt.headers.get("x-runlark-sha256-verified"), "true");
    assert.equal(await verifiedArt.text(), casPayload);
}
