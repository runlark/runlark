import { JSDOM } from "jsdom";

export const EVID_DIGEST = "deadbeef".repeat(8);
export const DIFF_DIGEST = "cafebabe".repeat(8);
export const CORRUPT_DIGEST = "baadf00d".repeat(8);
export const MISSING_DIGEST = "11223344".repeat(8);

export function setControlValue(win, el, value) {
    let proto = win.HTMLInputElement.prototype;
    if (el instanceof win.HTMLTextAreaElement) {
        proto = win.HTMLTextAreaElement.prototype;
    } else if (el instanceof win.HTMLSelectElement) {
        proto = win.HTMLSelectElement.prototype;
    }
    Object.getOwnPropertyDescriptor(proto, "value").set.call(el, value);
    el.dispatchEvent(new win.Event("input", { bubbles: true }));
    el.dispatchEvent(new win.Event("change", { bubbles: true }));
}

export function createTeamMockState() {
    const budget = {
        max_attempts: 3,
        max_duration_ms: 1_800_000,
        max_spend_micro_usd: 5_000_000,
        allow_unknown_cost_repair: false,
    };
    const state = {
        offline: false,
        cursorSeq: 12,
        sessions: [
            {
                server_url: "http://team.local:9880",
                user_id: "usr_alice",
                display_name: "Alice Owner",
                default_workspace_id: "ws_core",
                runner_id: "runner_a",
                runner_name: "Alice MacBook",
            },
        ],
        workspaces: [
            {
                workspace: {
                    id: "ws_core",
                    name: "Core Team",
                    slug: "core-team",
                    created_by: "usr_alice",
                    revision: 1,
                    created_at: 1000,
                    updated_at: 1000,
                },
                server_url: "http://team.local:9880",
                last_cursor_seq: 12,
            },
        ],
        members: [
            {
                workspace_id: "ws_core",
                user_id: "usr_alice",
                display_name: "Alice Owner",
                role: "owner",
                joined_at: 1000,
            },
            {
                workspace_id: "ws_core",
                user_id: "usr_bob",
                display_name: "Bob Dev",
                role: "member",
                joined_at: 1005,
            },
        ],
        invitations: [
            {
                id: "inv_seed_1",
                workspace_id: "ws_core",
                invitee_hint: "carol@team.dev",
                role: "viewer",
                status: "pending",
                invited_by: "usr_alice",
                expires_at: 9_999_999,
                created_at: 1010,
                accepted_at: null,
                accepted_by: null,
            },
        ],
        runners: [
            {
                id: "runner_a",
                workspace_id: "ws_core",
                owner_user_id: "usr_alice",
                name: "Alice MacBook",
                status: "online",
                last_heartbeat_at: 2000,
                created_at: 1000,
            },
            {
                id: "runner_b",
                workspace_id: "ws_core",
                owner_user_id: "usr_bob",
                name: "Bob Linux",
                status: "unreachable",
                last_heartbeat_at: 1200,
                created_at: 1000,
            },
        ],
        projects: [
            {
                project_id: "proj_team",
                name: "Team Repo",
                repo_root: "/tmp/team-repo",
                default_branch: "main",
                max_concurrent_runs: 2,
                policy_digest: "pol_team_1",
                budget_policy: budget,
                ownership: {
                    mode: "team_managed",
                    workspace_id: "ws_core",
                    server_url: "http://team.local:9880",
                    ownership_epoch: 1,
                },
                last_cursor_seq: 12,
                revision: 2,
                created_at: 1000,
                updated_at: 1000,
            },
            {
                project_id: "proj_solo",
                name: "Solo Repo",
                repo_root: "/tmp/solo-repo",
                default_branch: "main",
                max_concurrent_runs: 2,
                policy_digest: "pol_solo_1",
                budget_policy: budget,
                ownership: { mode: "solo_local" },
                last_cursor_seq: 0,
                revision: 1,
                created_at: 1000,
                updated_at: 1000,
            },
        ],
        soloHasActiveRun: true,
        entriesByProject: {
            proj_team: [
                {
                    task: {
                        id: "task_stuck",
                        project_id: "proj_team",
                        title: "Fix runner fencing",
                        description: "Run owned by unreachable runner",
                        kind: "standard",
                        stage: "in_progress",
                        workflow_stage: "implementation",
                        plan_version: 1,
                        plan_text: "Initial plan",
                        active_run_id: "run_stuck",
                        latest_run_id: "run_stuck",
                        active_session_id: "sess_stuck",
                        base_ref: "main",
                        head_ref: "runlark/task_stuck",
                        pr_url: null,
                        revision: 4,
                        created_at: 1000,
                        updated_at: 1000,
                    },
                    latest_run: {
                        id: "run_stuck",
                        session_id: "sess_stuck",
                        status: "Running",
                        worktree_path: "",
                        revision: 3,
                        task_id: "task_stuck",
                        project_id: "proj_team",
                        workflow_stage: "implementation",
                        task_kind: "standard",
                        workflow_digest: "wf_sha",
                        base_sha: "aaaa1111",
                        head_sha: "bbbb2222",
                        plan_version: 1,
                        policy_digest: "pol_team_1",
                        budget_policy: budget,
                        cost_summary: {
                            known_micro_usd: 10_000,
                            held_micro_usd: 0,
                            unknown_attempts: 0,
                        },
                        repair_attempts: 0,
                        pr_url: null,
                        created_at: 1000,
                        updated_at: 1000,
                    },
                    latest_run_ownership: {
                        assigned_runner_id: "runner_b",
                        lease_epoch: 2,
                        runner_unreachable: true,
                    },
                    session_revision: 5,
                    attempts: [],
                    cost_summary: {
                        known_micro_usd: 10_000,
                        held_micro_usd: 0,
                        unknown_attempts: 0,
                    },
                    decisions: [],
                    comments: [
                        {
                            id: "cmt_1",
                            task_id: "task_stuck",
                            run_id: "run_stuck",
                            author_user_id: "usr_alice",
                            author_display_name: "Alice Owner",
                            body: "Checked lease epoch on runner B.",
                            created_at: 1500,
                        },
                    ],
                    artifacts: [],
                    pending_questions: [],
                },
                {
                    task: {
                        id: "task_accept",
                        project_id: "proj_team",
                        title: "Ship CAS evidence",
                        description: "Ready for mobile acceptance",
                        kind: "quick_fix",
                        stage: "acceptance",
                        workflow_stage: "human_acceptance",
                        plan_version: 2,
                        plan_text: "Verified plan",
                        active_run_id: null,
                        latest_run_id: "run_accept",
                        active_session_id: "sess_accept",
                        base_ref: "main",
                        head_ref: "runlark/task_accept",
                        pr_url: null,
                        revision: 5,
                        created_at: 1000,
                        updated_at: 1000,
                    },
                    latest_run: {
                        id: "run_accept",
                        session_id: "sess_accept",
                        status: "WaitingForApproval",
                        worktree_path: "",
                        revision: 4,
                        task_id: "task_accept",
                        project_id: "proj_team",
                        workflow_stage: "human_acceptance",
                        task_kind: "quick_fix",
                        workflow_digest: "wf_sha",
                        base_sha: "1111aaaa",
                        head_sha: "2222bbbb",
                        plan_version: 2,
                        policy_digest: "pol_team_1",
                        budget_policy: budget,
                        cost_summary: {
                            known_micro_usd: 25_000,
                            held_micro_usd: 0,
                            unknown_attempts: 1,
                        },
                        repair_attempts: 0,
                        pr_url: null,
                        created_at: 1000,
                        updated_at: 1000,
                    },
                    latest_run_ownership: null,
                    session_revision: 6,
                    attempts: [],
                    cost_summary: {
                        known_micro_usd: 25_000,
                        held_micro_usd: 0,
                        unknown_attempts: 1,
                    },
                    decisions: [],
                    comments: [],
                    artifacts: [
                        {
                            digest_sha256: EVID_DIGEST,
                            kind: "evidence_bundle",
                            media_type: "application/json",
                            size_bytes: 64,
                            label: "evidence_bundle",
                            created_at: 1600,
                        },
                        {
                            digest_sha256: MISSING_DIGEST,
                            kind: "verification_log",
                            media_type: "text/plain",
                            size_bytes: 40,
                            label: "verification_log",
                            created_at: 1601,
                        },
                        {
                            digest_sha256: CORRUPT_DIGEST,
                            kind: "diff",
                            media_type: "text/x-diff",
                            size_bytes: 32,
                            label: "corrupt_diff",
                            created_at: 1602,
                        },
                    ],
                    pending_questions: [],
                },
            ],
            proj_solo: [
                {
                    task: {
                        id: "task_solo_1",
                        project_id: "proj_solo",
                        title: "Local solo job",
                        description: "",
                        kind: "quick_fix",
                        stage: "in_progress",
                        workflow_stage: "implementation",
                        plan_version: 1,
                        plan_text: null,
                        active_run_id: "run_solo_active",
                        latest_run_id: "run_solo_active",
                        active_session_id: "sess_solo",
                        base_ref: "main",
                        head_ref: null,
                        pr_url: null,
                        revision: 1,
                        created_at: 1000,
                        updated_at: 1000,
                    },
                    latest_run: {
                        id: "run_solo_active",
                        session_id: "sess_solo",
                        status: "Running",
                        worktree_path: "/tmp/wt/solo",
                        revision: 1,
                        task_id: "task_solo_1",
                        project_id: "proj_solo",
                        workflow_stage: "implementation",
                        task_kind: "quick_fix",
                        workflow_digest: null,
                        base_sha: null,
                        head_sha: null,
                        plan_version: 1,
                        policy_digest: "pol_solo_1",
                        budget_policy: budget,
                        cost_summary: {
                            known_micro_usd: 0,
                            held_micro_usd: 0,
                            unknown_attempts: 0,
                        },
                        repair_attempts: 0,
                        pr_url: null,
                        created_at: 1000,
                        updated_at: 1000,
                    },
                    latest_run_ownership: null,
                    session_revision: 1,
                    attempts: [],
                    cost_summary: { known_micro_usd: 0, held_micro_usd: 0, unknown_attempts: 0 },
                    decisions: [],
                    comments: [],
                    artifacts: [],
                    pending_questions: [],
                },
            ],
        },
        drafts: [
            {
                id: "draft_conflict_1",
                project_id: "proj_team",
                task_id: "task_stuck",
                kind: "plan_edit",
                payload_json: JSON.stringify({ plan_text: "Rebased plan v2" }),
                base_revision: 2,
                sync_state: {
                    state: "conflict",
                    server_revision: 4,
                    reason: "Revision mismatch: expected 2, server at 4",
                },
                created_at: 1700,
                updated_at: 1700,
            },
        ],
        calls: [],
    };
    return state;
}

export async function mountTeamDom(baseUrl, mobileModeRef) {
    const dom = new JSDOM(
        `<!DOCTYPE html><html lang="en"><body><div id="root"></div></body></html>`,
        {
            url: `${baseUrl}/`,
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
    dom.window.matchMedia = (query) => ({
        matches: query.includes("max-width") ? mobileModeRef.value : false,
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
    const { loadBundle, waitForUi } = await import("./ui-board-fixtures.mjs");
    const React = (await import("react")).default;
    const { act } = await import("react");
    const { createRoot } = await import("react-dom/client");
    const uiPlugin = loadBundle(dom.window);
    return {
        dom,
        uiPlugin,
        React,
        act,
        createRoot,
        waitForUi,
        rootEl: dom.window.document.getElementById("root"),
        restore() {
            dom.window.close();
            Object.assign(globalThis, prev);
        },
    };
}
