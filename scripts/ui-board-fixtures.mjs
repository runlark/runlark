import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import React, { act } from "react";
import ReactDom from "react-dom";
import * as ReactDomClient from "react-dom/client";
import * as ReactJsxRuntime from "react/jsx-runtime";

const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const CLIENT_BUNDLE = join(REPO_ROOT, "packages", "dsh-plugin", "dist", "client.js");
const require = createRequire(import.meta.url);

export function loadBundle(win) {
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

export async function waitForUi(check, timeoutMs = 4000) {
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

export function createBoardState() {
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
                attempt_no: 1,
                workflow_stage: wf,
                outcome: { status: "succeeded" },
                cost_micro_usd: unknownAttempts > 0 ? null : 15_000,
                unknown_cost_reason: unknownAttempts > 0 ? "missing provider pricing" : null,
                started_at: 1000,
                ended_at: 1100,
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
