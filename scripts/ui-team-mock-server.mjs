import http from "node:http";
import { CORRUPT_DIGEST, DIFF_DIGEST, EVID_DIGEST, MISSING_DIGEST } from "./ui-team-fixtures.mjs";

export function startTeamMockServer(state) {
    return http.createServer(async (req, res) => {
        const url = new URL(req.url ?? "/", "http://127.0.0.1");
        const chunks = [];
        for await (const c of req) chunks.push(c);
        const body = chunks.length ? JSON.parse(Buffer.concat(chunks).toString("utf8")) : {};
        state.calls.push({ method: req.method, path: url.pathname, body });

        const send = (code, data) => {
            res.writeHead(code, { "Content-Type": "application/json" });
            res.end(JSON.stringify(data));
        };

        if (req.method === "GET" && url.pathname === "/api/runlark/projects") {
            return send(200, state.projects);
        }
        if (req.method === "GET" && url.pathname === "/api/runlark/plugins") {
            return send(200, []);
        }
        if (req.method === "GET" && url.pathname === "/api/runlark/board") {
            const pid = url.searchParams.get("project_id") ?? "proj_team";
            const project = state.projects.find((p) => p.project_id === pid) ?? state.projects[0];
            const entries = state.entriesByProject[project.project_id] ?? [];
            const columns = { inbox: [], ready: [], in_progress: [], acceptance: [], done: [] };
            const activeRuns = [];
            const runOwnerships = {};
            for (const e of entries) {
                columns[e.task.stage].push(e);
                if (e.latest_run && e.latest_run.status === "Running")
                    activeRuns.push(e.latest_run);
                if (e.latest_run && e.latest_run_ownership) {
                    runOwnerships[e.latest_run.id] = e.latest_run_ownership;
                }
            }
            return send(200, {
                project,
                ownership_mode: project.ownership,
                last_cursor_seq: state.cursorSeq,
                columns,
                tasks: entries,
                active_runs: activeRuns,
                queued_runs: [],
                attempts: [],
                cost_summary: { known_micro_usd: 35_000, held_micro_usd: 0, unknown_attempts: 1 },
                decisions: [],
                pending_questions: [],
                pending_drafts: state.drafts,
                runners: state.runners,
                members: state.members,
                run_ownerships: runOwnerships,
                slots: {
                    max: 2,
                    used: activeRuns.length,
                    available: Math.max(0, 2 - activeRuns.length),
                },
            });
        }
        const taskGet = /^\/api\/runlark\/tasks\/([^/]+)$/u.exec(url.pathname);
        if (req.method === "GET" && taskGet) {
            const all = Object.values(state.entriesByProject).flat();
            const found = all.find((e) => e.task.id === taskGet[1]);
            return found ? send(200, found) : send(404, { error: "not_found" });
        }
        if (req.method === "GET" && url.pathname.endsWith("/history")) {
            const rid = url.pathname.split("/")[4];
            const all = Object.values(state.entriesByProject).flat();
            const e = all.find((x) => x.latest_run?.id === rid);
            if (!e) {
                return send(404, { error: "not_found" });
            }
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
                run_id: "run_accept",
                base_sha: "1111aaaa",
                head_sha: "2222bbbb",
                branch_name: "runlark/task_accept",
                changed_files: ["src/team.rs"],
                diff: "+verified team diff\n",
                diff_bytes: 20,
                digest_sha256: DIFF_DIGEST,
            });
        }
        if (req.method === "GET" && url.pathname === `/api/runlark/artifacts/${EVID_DIGEST}`) {
            res.writeHead(200, {
                "Content-Type": "application/json",
                "X-Runlark-Sha256-Verified": "true",
            });
            res.end(JSON.stringify({ verdict: "pass", checks: ["clippy", "wasm"] }));
            return;
        }
        if (req.method === "GET" && url.pathname === `/api/runlark/artifacts/${MISSING_DIGEST}`) {
            return send(404, { error: "artifact_not_found", message: "Blob not synced locally" });
        }
        if (req.method === "GET" && url.pathname === `/api/runlark/artifacts/${CORRUPT_DIGEST}`) {
            return send(500, {
                error: "corrupt_artifact",
                message: "SHA-256 mismatch on disk",
            });
        }
        if (req.method === "GET" && url.pathname === "/api/runlark/team/status") {
            return send(200, {
                sessions: state.sessions,
                workspaces: state.workspaces,
                project_bindings: [],
                unresolved_drafts_count: state.drafts.filter((d) => d.sync_state.state !== "synced")
                    .length,
                pending_outbox_count: 0,
                conflict_outbox_count: state.drafts.filter((d) => d.sync_state.state === "conflict")
                    .length,
            });
        }
        if (req.method === "POST" && url.pathname === "/api/runlark/team/sync") {
            if (state.offline) {
                return send(503, { error: "team_server_offline", message: "Server offline" });
            }
            state.cursorSeq += 1;
            return send(200, {
                workspaces_synced: 1,
                snapshots_applied: 0,
                events_applied: 2,
                outbox_acked: 1,
                outbox_conflicts: state.drafts.filter((d) => d.sync_state.state === "conflict")
                    .length,
                cas_uploaded: 0,
                cas_downloaded: 1,
            });
        }
        if (req.method === "GET" && url.pathname === "/api/runlark/team/workspaces/ws_core") {
            return send(200, { workspace: state.workspaces[0].workspace, members: state.members });
        }
        if (
            req.method === "PATCH" &&
            url.pathname === "/api/runlark/team/workspaces/ws_core/members/usr_bob"
        ) {
            const m = state.members.find((x) => x.user_id === "usr_bob");
            m.role = body.role;
            return send(200, m);
        }
        if (
            req.method === "GET" &&
            url.pathname === "/api/runlark/team/workspaces/ws_core/invitations"
        ) {
            return send(200, state.invitations);
        }
        if (
            req.method === "POST" &&
            url.pathname === "/api/runlark/team/workspaces/ws_core/invitations"
        ) {
            const inv = {
                id: "inv_new_2",
                workspace_id: "ws_core",
                invitee_hint: body.invitee_hint,
                role: body.role,
                status: "pending",
                invited_by: "usr_alice",
                expires_at: 9_999_999,
                created_at: 2000,
                accepted_at: null,
                accepted_by: null,
            };
            state.invitations.unshift(inv);
            return send(201, { invitation: inv, invite_token: "inv_secret_once_xyz" });
        }
        const revokeMatch =
            /^\/api\/runlark\/team\/workspaces\/ws_core\/invitations\/([^/]+)\/revoke$/u.exec(
                url.pathname,
            );
        if (req.method === "POST" && revokeMatch) {
            const inv = state.invitations.find((i) => i.id === revokeMatch[1]);
            inv.status = "revoked";
            return send(200, inv);
        }
        if (req.method === "GET" && url.pathname === "/api/runlark/team/runners/status") {
            return send(200, {
                workspace_id: "ws_core",
                local_runner_id: "runner_a",
                local_runner_name: "Alice MacBook",
                buffered_events_count: 0,
                workspace_runners: state.runners,
            });
        }
        if (req.method === "GET" && url.pathname === "/api/runlark/team/drafts") {
            return send(200, { drafts: state.drafts, outbox: [] });
        }
        const retryMatch = /^\/api\/runlark\/team\/drafts\/([^/]+)\/retry$/u.exec(url.pathname);
        if (req.method === "POST" && retryMatch) {
            const d = state.drafts.find((x) => x.id === retryMatch[1]);
            d.sync_state = { state: "synced" };
            d.updated_at = Date.now();
            return send(200, d);
        }
        const delDraftMatch = /^\/api\/runlark\/team\/drafts\/([^/]+)$/u.exec(url.pathname);
        if (req.method === "DELETE" && delDraftMatch) {
            state.drafts = state.drafts.filter((x) => x.id !== delDraftMatch[1]);
            res.writeHead(204);
            res.end();
            return;
        }
        if (
            req.method === "POST" &&
            url.pathname === "/api/runlark/projects/proj_solo/promote-team"
        ) {
            if (state.soloHasActiveRun) {
                return send(409, {
                    error: "team_conflict",
                    server_code: "solo_to_team_blocked",
                    message: "Active runs prevent promotion",
                });
            }
            const solo = state.projects.find((p) => p.project_id === "proj_solo");
            solo.ownership = {
                mode: "team_managed",
                workspace_id: body.workspace_id,
                server_url: "http://team.local:9880",
                ownership_epoch: 1,
            };
            return send(200, {
                ack: {
                    workspace_id: body.workspace_id,
                    project_id: "proj_solo",
                    initial_cursor: 13,
                    ownership_epoch: 1,
                    acknowledged_at: 2100,
                },
                binding: {
                    project_id: "proj_solo",
                    ownership_mode: solo.ownership,
                    last_cursor_seq: 13,
                    updated_at: 2100,
                },
            });
        }
        if (
            req.method === "POST" &&
            url.pathname === "/api/runlark/team/runs/run_stuck/cancel-unreachable"
        ) {
            const e = state.entriesByProject.proj_team[0];
            e.latest_run = {
                ...e.latest_run,
                status: "Stopped",
                revision: e.latest_run.revision + 1,
            };
            e.latest_run_ownership = { ...e.latest_run_ownership, runner_unreachable: false };
            return send(200, {
                command_id: "cmd_cancel_1",
                idempotent_replay: false,
                cursor_seq: ++state.cursorSeq,
                aggregate_type: "run",
                aggregate_id: "run_stuck",
                revision: e.latest_run.revision,
            });
        }
        if (req.method === "POST" && url.pathname === "/api/runlark/tasks/task_stuck/comments") {
            if (state.offline) {
                const draft = {
                    id: "draft_cmt_offline",
                    project_id: "proj_team",
                    task_id: "task_stuck",
                    kind: "task_comment",
                    payload_json: JSON.stringify({ body: body.body }),
                    base_revision: null,
                    sync_state: { state: "pending_sync" },
                    created_at: 2200,
                    updated_at: 2200,
                };
                state.drafts.push(draft);
                return send(202, { delivery: "queued_offline", draft });
            }
            const cmt = {
                id: `cmt_${state.entriesByProject.proj_team[0].comments.length + 1}`,
                task_id: "task_stuck",
                run_id: body.run_id ?? null,
                author_user_id: "usr_alice",
                author_display_name: "Alice Owner",
                body: body.body,
                created_at: 2200,
            };
            state.entriesByProject.proj_team[0].comments.push(cmt);
            return send(201, { delivery: "synced", comment: cmt });
        }
        if (req.method === "POST" && url.pathname === "/api/runlark/tasks/task_accept/accept") {
            const e = state.entriesByProject.proj_team[1];
            e.task = { ...e.task, stage: "done", revision: e.task.revision + 1 };
            e.decisions.push({
                id: "dec_mobile_1",
                task_id: "task_accept",
                run_id: "run_accept",
                kind: "accept",
                actor: "usr_alice",
                plan_version: body.plan_version,
                head_sha: body.head_sha,
                base_sha: body.base_sha,
                policy_digest: body.policy_digest,
                evidence_digest: body.evidence_digest,
                comment: body.comment ?? null,
                created_at: 2300,
            });
            return send(200, { decision_id: "dec_mobile_1", task: e.task, run: e.latest_run });
        }
        return send(404, { error: "not_found" });
    });
}
