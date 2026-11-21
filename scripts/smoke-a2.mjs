import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
    bootstrapTeamOwner,
    jsonPost,
    startTeamServer,
    verifyMountedTeamUi,
    verifyOfflineConflictAndCas,
} from "./smoke-a2-helpers.mjs";
import {
    apiJson,
    getFreePort,
    initGitRepo,
    queryDb,
    startDaemon,
    startDshHost,
    stopProcess,
    waitFor,
} from "./smoke-harness.mjs";

export async function runA2TeamSmoke(binaryPath, serverBin) {
    const postgresUrl = process.env.RUNLARK_TEST_POSTGRES_URL;
    if (!postgresUrl) {
        throw new Error(
            "RUNLARK_TEST_POSTGRES_URL is required for A2 real PostgreSQL E2E smoke (run via scripts/with-test-postgres.mjs)",
        );
    }

    console.log(
        "Running A2 team E2E smoke (real runlark-server/PostgreSQL + 2 daemons/runners, solo->team, sync, fencing, 409, offline drafts, reconnect, CAS)...",
    );

    const suffix = `${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 6)}`;
    const baseDir = mkdtempSync(join(tmpdir(), "runlark-a2-smoke-"));
    const casRoot = join(baseDir, "server-cas");
    const repo1 = join(baseDir, "repo-alice");
    const data1 = join(baseDir, "daemon-alice");
    const dsh1Dir = join(baseDir, "dsh-alice");
    const data2 = join(baseDir, "daemon-bob");

    const serverPort = await getFreePort();
    const daemon1Port = await getFreePort();
    const dsh1Port = await getFreePort();
    const daemon2Port = await getFreePort();

    let teamSrv = null;
    let dsh1 = null;
    let daemon1 = null;
    let daemon2 = null;

    try {
        for (const dir of [casRoot, repo1, data1, dsh1Dir, data2]) {
            mkdirSync(dir, { recursive: true });
        }
        initGitRepo(repo1);

        const wsId = `ws_a2_${suffix}`;
        const aliceUid = `usr_alice_${suffix}`;
        const bobUid = `usr_bob_${suffix}`;
        const projId = `proj_a2_${suffix}`;

        const ownerBoot = bootstrapTeamOwner(serverBin, postgresUrl, {
            userId: aliceUid,
            displayName: "Alice Smoke",
            workspaceId: wsId,
            workspaceName: "A2 Smoke Team",
            workspaceSlug: `a2-smoke-${suffix}`,
        });
        assert.equal(ownerBoot.workspace_id, wsId);

        teamSrv = await startTeamServer(serverBin, postgresUrl, casRoot, serverPort, 1200);
        const { serverUrl } = teamSrv;
        dsh1 = await startDshHost(`http://127.0.0.1:${daemon1Port}`, dsh1Dir, dsh1Port);
        daemon1 = await startDaemon(binaryPath, data1, daemon1Port, dsh1.url);
        daemon2 = await startDaemon(binaryPath, data2, daemon2Port, null);

        const dshOrigin = dsh1.origin;
        const d2Url = daemon2.daemonUrl;

        // 1. Auth, Workspace Roles & Invitations (create, list, revoke, accept, role change)
        const login1 = await apiJson(
            `${dshOrigin}/api/runlark/team/login`,
            jsonPost({
                server_url: serverUrl,
                bearer_token: ownerBoot.bearer_token,
                workspace_id: wsId,
            }),
        );
        assert.equal(login1.status, 200);
        assert.equal(login1.json.user_id, aliceUid);

        const invBob = await apiJson(
            `${dshOrigin}/api/runlark/team/workspaces/${wsId}/invitations`,
            jsonPost({ invitee_hint: "bob@smoke.dev", role: "member" }),
        );
        assert.equal(invBob.status, 201);
        const invRevokeTarget = await apiJson(
            `${dshOrigin}/api/runlark/team/workspaces/${wsId}/invitations`,
            jsonPost({ invitee_hint: "temp@smoke.dev", role: "viewer" }),
        );
        assert.equal(invRevokeTarget.status, 201);

        const invList = await apiJson(
            `${dshOrigin}/api/runlark/team/workspaces/${wsId}/invitations`,
        );
        assert.equal(invList.status, 200);
        assert.equal(invList.json.length, 2);

        const revoked = await apiJson(
            `${dshOrigin}/api/runlark/team/workspaces/${wsId}/invitations/${invRevokeTarget.json.invitation.id}/revoke`,
            jsonPost({}),
        );
        assert.equal(revoked.status, 200);
        assert.equal(revoked.json.status, "revoked");

        const accBob = await apiJson(
            `${d2Url}/api/v1/team/invitations/accept`,
            jsonPost({
                server_url: serverUrl,
                invite_token: invBob.json.invite_token,
                user_id: bobUid,
                display_name: "Bob Smoke",
            }),
        );
        assert.equal(accBob.status, 200);
        assert.equal(accBob.json.token_saved, true);

        const rolePatch = await apiJson(
            `${dshOrigin}/api/runlark/team/workspaces/${wsId}/members/${bobUid}`,
            {
                method: "PATCH",
                headers: { "Content-Type": "application/json" },
                body: JSON.stringify({ role: "maintainer" }),
            },
        );
        assert.equal(rolePatch.status, 200);
        assert.equal(rolePatch.json.role, "maintainer");

        // 2. Solo -> Team promotion blocked while active run exists, succeeds after run settles
        const projRes = await apiJson(
            `${dshOrigin}/api/runlark/projects`,
            jsonPost({
                project_id: projId,
                name: "A2 Promoted Repo",
                repo_root: repo1,
                default_branch: "main",
            }),
        );
        assert.equal(projRes.status, 201);

        const tPromo = await apiJson(
            `${dshOrigin}/api/runlark/tasks`,
            jsonPost({
                project_id: projId,
                title: "A2 Sync & Conflict Task",
                description: "Promoted from solo to team",
                kind: "quick_fix",
            }),
        );
        assert.equal(tPromo.status, 201);
        const taskId = tPromo.json.id;

        const activeSoloStart = await apiJson(
            `${dshOrigin}/api/runlark/tasks/${taskId}/start`,
            jsonPost({ prompt: "WAIT_CANCEL block solo promotion" }),
        );
        assert.equal(activeSoloStart.status, 201);
        assert.equal(activeSoloStart.json.admitted, true);

        const blockedPromote = await apiJson(
            `${dshOrigin}/api/runlark/projects/${projId}/promote-team`,
            jsonPost({ workspace_id: wsId }),
        );
        assert.equal(
            blockedPromote.status,
            409,
            "solo->team must fail with 409 when active run exists",
        );

        const soloSid = activeSoloStart.json.run.session_id;
        await waitFor(
            () => {
                const rows = queryDb(
                    join(data1, "runlark.db"),
                    "SELECT status FROM commands WHERE session_id = ? AND status = 'acked'",
                    soloSid,
                );
                return rows.length > 0;
            },
            6000,
            "solo run SendPrompt outbox acked by DSH before stop",
        );

        const stopSolo = await apiJson(
            `${dshOrigin}/api/runlark/sessions/${soloSid}/stop`,
            jsonPost({}),
        );
        assert.equal(stopSolo.status, 200, `stopSolo failed: ${stopSolo.text}`);
        await waitFor(
            async () => {
                const s = await apiJson(`${daemon1.daemonUrl}/api/v1/sessions/${soloSid}`);
                return (
                    s.ok && s.json.active_run_id === null && s.json.latest_run_status === "stopped"
                );
            },
            6000,
            "solo run settles to stopped before promotion",
        );

        const promoted = await apiJson(
            `${dshOrigin}/api/runlark/projects/${projId}/promote-team`,
            jsonPost({ workspace_id: wsId }),
        );
        assert.equal(promoted.status, 200);
        assert.equal(promoted.json.ack.workspace_id, wsId);
        assert.equal(promoted.json.ack.ownership_epoch, 1);
        assert.equal(promoted.json.binding.ownership_mode.mode, "team_managed");

        // 3. Snapshot + incremental cursor/events sync across Daemon #1 and Daemon #2
        const syncBob1 = await apiJson(
            `${d2Url}/api/v1/team/sync`,
            jsonPost({ workspace_id: wsId }),
        );
        assert.equal(syncBob1.status, 200);
        const bobBoard1 = await apiJson(`${d2Url}/api/v1/board?project_id=${projId}`);
        assert.equal(bobBoard1.status, 200);
        assert.equal(bobBoard1.json.ownership_mode.mode, "team_managed");
        assert.ok(bobBoard1.json.last_cursor_seq > 0);
        assert.ok(bobBoard1.json.tasks.some((e) => e.task.id === taskId));

        const cmt1 = await apiJson(
            `${dshOrigin}/api/runlark/tasks/${taskId}/comments`,
            jsonPost({ body: "Online comment from Alice" }),
        );
        assert.equal(cmt1.status, 201);
        assert.equal(cmt1.json.delivery, "synced");

        const syncBob2 = await apiJson(
            `${d2Url}/api/v1/team/sync`,
            jsonPost({ workspace_id: wsId }),
        );
        assert.equal(syncBob2.status, 200);
        assert.equal(syncBob2.json.snapshots_applied, 0, "incremental sync must use cursor events");
        assert.ok(syncBob2.json.events_applied >= 1);
        const bobComments1 = await apiJson(`${d2Url}/api/v1/tasks/${taskId}/comments`);
        assert.ok(bobComments1.json.some((c) => c.body === "Online comment from Alice"));

        // 4. Exclusive Runner Assignment, Writer-Side Fencing (lease_epoch), No Auto-Takeover, & Cancel-Unreachable
        const teamTaskRes = await apiJson(
            `${dshOrigin}/api/runlark/tasks`,
            jsonPost({
                project_id: projId,
                title: "A2 Runner Fencing Task",
                description: "Tests exclusive lease epoch and cancel-unreachable",
                kind: "quick_fix",
            }),
        );
        assert.equal(teamTaskRes.status, 201);
        const fencingTaskId = teamTaskRes.json.id;

        // Start before registering runners so run enters Queued unassigned, then explicitly assign Runner A
        const teamStart = await apiJson(
            `${dshOrigin}/api/runlark/tasks/${fencingTaskId}/start`,
            jsonPost({ prompt: "WAIT_CANCEL team fenced run" }),
        );
        assert.equal(teamStart.status, 201, `teamStart failed: ${teamStart.text}`);
        const teamRunId = teamStart.json.run.id;

        const regA = await apiJson(
            `${dshOrigin}/api/runlark/team/runners/register`,
            jsonPost({ workspace_id: wsId, name: "Runner Alice" }),
        );
        assert.equal(regA.status, 200, `regA failed: ${regA.text}`);
        const regB = await apiJson(
            `${d2Url}/api/v1/team/runners/register`,
            jsonPost({ workspace_id: wsId, name: "Runner Bob" }),
        );
        assert.equal(regB.status, 200, `regB failed: ${regB.text}`);
        const runnerAId = regA.json.id;
        const runnerBId = regB.json.id;

        const assignA = await apiJson(
            `${dshOrigin}/api/runlark/team/runs/${teamRunId}/assign`,
            jsonPost({ workspace_id: wsId, runner_id: runnerAId }),
        );
        assert.equal(assignA.status, 200, `assignA failed: ${assignA.text}`);
        const leaseEpoch = assignA.json.ownership.lease_epoch;
        assert.equal(assignA.json.ownership.assigned_runner_id, runnerAId);
        assert.equal(leaseEpoch, 1);

        // No silent auto-takeover: assigning Runner B to Runner A's active run must 409
        const takeoverAttempt = await apiJson(
            `${d2Url}/api/v1/team/runs/${teamRunId}/assign`,
            jsonPost({ workspace_id: wsId, runner_id: runnerBId }),
        );
        assert.equal(
            takeoverAttempt.status,
            409,
            "active run must block takeover by another runner",
        );

        // Writer-side fencing & runner ownership authz on runlark-server:
        // (a) Alice spoofing Bob's runner_id must 403; (b) Bob with Bob's token + runnerBId must 409;
        // (c) Alice with wrong lease_epoch must 409.
        const bobTokenRows = queryDb(
            join(data2, "runlark.db"),
            "SELECT bearer_token FROM team_auth_sessions WHERE server_url = ?",
            serverUrl,
        );
        const bobBearerToken = bobTokenRows[0]?.bearer_token;
        assert.ok(bobBearerToken, "Bob bearer token must be persisted in daemon2 SQLite");

        const spoofRunner = await apiJson(
            `${serverUrl}/api/v1/workspaces/${wsId}/runs/${teamRunId}/runner-state`,
            {
                method: "POST",
                headers: {
                    "Content-Type": "application/json",
                    Authorization: `Bearer ${ownerBoot.bearer_token}`,
                },
                body: JSON.stringify({
                    runner_id: runnerBId,
                    lease_epoch: leaseEpoch,
                    status: "Running",
                    events: [],
                }),
            },
        );
        assert.equal(spoofRunner.status, 403, "impersonating another user's runner must 403");

        const fenceWrongRunner = await apiJson(
            `${serverUrl}/api/v1/workspaces/${wsId}/runs/${teamRunId}/runner-state`,
            {
                method: "POST",
                headers: {
                    "Content-Type": "application/json",
                    Authorization: `Bearer ${bobBearerToken}`,
                },
                body: JSON.stringify({
                    runner_id: runnerBId,
                    lease_epoch: leaseEpoch,
                    status: "Running",
                    events: [],
                }),
            },
        );
        assert.equal(
            fenceWrongRunner.status,
            409,
            `wrong runner_id must fail writer fencing: ${fenceWrongRunner.text}`,
        );

        const fenceWrongEpoch = await apiJson(
            `${serverUrl}/api/v1/workspaces/${wsId}/runs/${teamRunId}/runner-state`,
            {
                method: "POST",
                headers: {
                    "Content-Type": "application/json",
                    Authorization: `Bearer ${ownerBoot.bearer_token}`,
                },
                body: JSON.stringify({
                    runner_id: runnerAId,
                    lease_epoch: leaseEpoch + 5,
                    status: "Running",
                    events: [],
                }),
            },
        );
        assert.equal(fenceWrongEpoch.status, 409, "wrong lease_epoch must fail writer fencing");

        // Stop Daemon #1 so Runner A stops heartbeating -> sweep marks Runner A unreachable
        await stopProcess(daemon1, "a2-daemon-alice-stop");
        await new Promise((r) => setTimeout(r, 1500));
        const sweepRes = await apiJson(`${serverUrl}/api/v1/workspaces/${wsId}/runners/sweep`, {
            method: "POST",
            headers: { Authorization: `Bearer ${ownerBoot.bearer_token}` },
        });
        assert.equal(sweepRes.status, 200);
        await apiJson(`${d2Url}/api/v1/team/sync`, jsonPost({ workspace_id: wsId }));
        const bobBoardUnreach = await apiJson(`${d2Url}/api/v1/board?project_id=${projId}`);
        assert.equal(
            bobBoardUnreach.json.run_ownerships[teamRunId]?.runner_unreachable,
            true,
            "run ownership must explicitly reflect runner_unreachable=true",
        );

        // Even while unreachable, auto-takeover/assign to Runner B must still be rejected (409)
        const takeoverWhileUnreach = await apiJson(
            `${d2Url}/api/v1/team/runs/${teamRunId}/assign`,
            jsonPost({ workspace_id: wsId, runner_id: runnerBId }),
        );
        assert.equal(takeoverWhileUnreach.status, 409);

        // Explicit cancel-unreachable succeeds without silent takeover
        const cancelUnreach = await apiJson(
            `${d2Url}/api/v1/team/runs/${teamRunId}/cancel-unreachable`,
            jsonPost({ workspace_id: wsId, reason: "Alice host offline during smoke" }),
        );
        assert.equal(cancelUnreach.status, 200);

        // Restart Daemon #1 so Alice is online again
        daemon1 = await startDaemon(binaryPath, data1, daemon1Port, dsh1.url);
        await apiJson(`${dshOrigin}/api/runlark/team/sync`, jsonPost({ workspace_id: wsId }));

        // 5-6. 409 conflict, server outage offline gate (503), offline drafts, reconnect replay & verified CAS
        const phaseCtx = {
            teamSrv,
            dshOrigin,
            d2Url,
            wsId,
            projId,
            taskId,
            teamRunId,
            suffix,
            ownerToken: ownerBoot.bearer_token,
            serverBin,
            postgresUrl,
            casRoot,
            serverPort,
        };
        try {
            await verifyOfflineConflictAndCas(phaseCtx);
        } finally {
            teamSrv = phaseCtx.teamSrv;
        }

        // 7. Mount real compiled Team UI against live DSH proxy #1
        await verifyMountedTeamUi(dshOrigin, "A2 Sync & Conflict Task");

        console.log(
            "A2 team E2E smoke PASSED: real PostgreSQL runlark-server + 2 daemons/runners, roles/invites, solo->team active-run guard, snapshot/cursor sync, lease_epoch fencing without auto-takeover, cancel-unreachable, 409 conflict, offline drafts, reconnect replay, verified CAS, and live Team UI verified.",
        );
    } finally {
        await stopProcess(daemon1, "a2-daemon1-final").catch(() => {});
        await stopProcess(daemon2, "a2-daemon2-final").catch(() => {});
        await stopProcess(dsh1, "a2-dsh1-final").catch(() => {});
        await stopProcess(teamSrv, "a2-server-final").catch(() => {});
        rmSync(baseDir, { recursive: true, force: true });
    }
}
