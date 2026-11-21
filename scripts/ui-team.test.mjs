import assert from "node:assert/strict";
import { test } from "node:test";
import {
    EVID_DIGEST,
    createTeamMockState,
    mountTeamDom,
    setControlValue,
} from "./ui-team-fixtures.mjs";
import { startTeamMockServer } from "./ui-team-mock-server.mjs";

test("Team UI: workspaces/roles/invites, solo->team active-run block, runner cancel-unreachable, comments/offline/409, CAS verify, mobile acceptance", async () => {
    const state = createTeamMockState();
    const server = startTeamMockServer(state);
    await new Promise((r) => server.listen(0, "127.0.0.1", r));
    const baseUrl = `http://127.0.0.1:${server.address().port}`;
    const mobileMode = { value: false };
    const env = await mountTeamDom(baseUrl, mobileMode);
    const { dom, uiPlugin, rootEl, React, act, createRoot, waitForUi } = env;
    const root = createRoot(rootEl);

    try {
        await act(async () => {
            root.render(React.createElement(uiPlugin.RunlarkBoardTabBody));
        });

        // 1. Sync status bar shows Team Managed mode, conflict count, and board card shows unreachable runner badge
        await waitForUi(() => {
            assert.ok(rootEl.querySelector('[data-testid="rl-team-sync-strip"]'));
            assert.ok(rootEl.querySelector('[data-testid="rl-sync-drafts-chip"]'));
            const cardRunner = rootEl.querySelector('[data-testid="rl-card-runner-task_stuck"]');
            assert.ok(cardRunner);
            assert.match(cardRunner.textContent, /unreachable/i);
        });

        // 2. Navigate to Team view: update member role, create & revoke invitation, rebase & retry 409 draft
        await act(async () => {
            rootEl.querySelector('[data-testid="rl-nav-team"]').click();
        });
        await waitForUi(() => {
            assert.ok(rootEl.querySelector('[data-testid="rl-team-workspace-view"]'));
            assert.ok(rootEl.querySelector('[data-testid="rl-member-role-usr_bob"]'));
            assert.ok(rootEl.querySelector('[data-testid="rl-draft-row-draft_conflict_1"]'));
        });

        // Sole Owner select is disabled with explanation
        const aliceSelect = rootEl.querySelector('[data-testid="rl-member-role-usr_alice"]');
        assert.equal(aliceSelect.disabled, true);
        assert.ok(aliceSelect.getAttribute("aria-describedby"));

        // Promote Bob member -> maintainer
        await act(async () => {
            setControlValue(
                dom.window,
                rootEl.querySelector('[data-testid="rl-member-role-usr_bob"]'),
                "maintainer",
            );
        });
        await waitForUi(() => {
            assert.equal(state.members.find((m) => m.user_id === "usr_bob").role, "maintainer");
            assert.equal(
                rootEl.querySelector('[data-testid="rl-create-invite-submit"]')?.disabled,
                false,
            );
        });

        // Create invitation -> displays one-time invite token; then revoke pending seed invitation
        await act(async () => {
            setControlValue(
                dom.window,
                rootEl.querySelector('[data-testid="rl-invite-hint-input"]'),
                "dave@team.dev",
            );
        });
        await act(async () => {
            rootEl.querySelector('[data-testid="rl-create-invite-submit"]').click();
        });
        await waitForUi(() => {
            assert.equal(
                rootEl.querySelector('[data-testid="rl-created-invite-token"]')?.textContent,
                "inv_secret_once_xyz",
            );
            assert.equal(
                rootEl.querySelector('[data-testid="rl-revoke-invite-inv_seed_1"]')?.disabled,
                false,
            );
        });
        await act(async () => {
            rootEl.querySelector('[data-testid="rl-revoke-invite-inv_seed_1"]').click();
        });
        await waitForUi(() => {
            assert.equal(state.invitations.find((i) => i.id === "inv_seed_1").status, "revoked");
        });

        // Rebase & retry conflicted 409 draft (pre-filled with server_revision = 4)
        const revInput = rootEl.querySelector('[data-testid="rl-draft-rev-draft_conflict_1"]');
        assert.equal(revInput.value, "4");
        await act(async () => {
            rootEl.querySelector('[data-testid="rl-draft-retry-draft_conflict_1"]').click();
        });
        await waitForUi(() => {
            assert.equal(state.drafts[0].sync_state.state, "synced");
        });

        // 3. Switch to Solo project -> open Projects -> verify active-run blocks Solo -> Team promotion
        await act(async () => {
            const projSelect = rootEl.querySelector(".rl-rail-select");
            setControlValue(dom.window, projSelect, "proj_solo");
        });
        await act(async () => {
            rootEl.querySelector('[data-testid="rl-nav-projects"]').click();
        });
        await waitForUi(() => {
            assert.ok(rootEl.querySelector('[data-testid="rl-promote-open-proj_solo"]'));
        });
        await act(async () => {
            rootEl.querySelector('[data-testid="rl-promote-open-proj_solo"]').click();
        });
        await waitForUi(() => {
            assert.ok(rootEl.querySelector('[data-testid="rl-promote-blocked-notice"]'));
            const confirmBtn = rootEl.querySelector('[data-testid="rl-promote-confirm-btn"]');
            assert.equal(confirmBtn.disabled, true);
            assert.ok(confirmBtn.getAttribute("aria-describedby"));
        });
        // Settle active solo run, refresh, and promote cleanly
        state.soloHasActiveRun = false;
        state.entriesByProject.proj_solo[0].latest_run.status = "Stopped";
        await act(async () => {
            rootEl.querySelector('[data-testid="rl-promote-cancel-btn"]').click();
            rootEl.querySelector('[data-testid="rl-nav-board"]').click();
        });
        await waitForUi(() => {
            assert.ok(rootEl.querySelector('[data-testid="rl-board-surface"]'));
        });
        await act(async () => {
            rootEl.querySelector('[data-testid="rl-nav-projects"]').click();
        });
        await waitForUi(() => {
            assert.ok(rootEl.querySelector('[data-testid="rl-promote-open-proj_solo"]'));
        });
        await act(async () => {
            rootEl.querySelector('[data-testid="rl-promote-open-proj_solo"]').click();
        });
        await waitForUi(() => {
            assert.equal(
                rootEl.querySelector('[data-testid="rl-promote-confirm-btn"]').disabled,
                false,
            );
        });
        await act(async () => {
            rootEl
                .querySelector('[data-testid="rl-promote-team-dialog"] form')
                .dispatchEvent(new dom.window.Event("submit", { bubbles: true, cancelable: true }));
        });
        await waitForUi(() => {
            const solo = state.projects.find((p) => p.project_id === "proj_solo");
            assert.equal(solo.ownership.mode, "team_managed");
        });

        // 4. Switch back to Team project -> open task_stuck -> Runner Ownership & cancel-unreachable (focus starts on Cancel)
        await act(async () => {
            setControlValue(dom.window, rootEl.querySelector(".rl-rail-select"), "proj_team");
            rootEl.querySelector('[data-testid="rl-nav-board"]').click();
        });
        await waitForUi(() => {
            assert.ok(rootEl.querySelector('[data-testid="rl-card-open-task_stuck"]'));
        });
        await act(async () => {
            rootEl.querySelector('[data-testid="rl-card-open-task_stuck"]').click();
        });
        await waitForUi(() => {
            assert.ok(rootEl.querySelector('[data-testid="rl-task-detail"]'));
        });
        await act(async () => {
            rootEl.querySelector('[data-testid="rl-task-tab-run_budget"]').click();
        });
        await waitForUi(() => {
            assert.ok(rootEl.querySelector('[data-testid="rl-runner-ownership-panel"]'));
            assert.ok(rootEl.querySelector('[data-testid="rl-cancel-unreachable-open-btn"]'));
            // Must NOT show runner reassignment controls on owned run
            assert.equal(rootEl.querySelector('[data-testid="rl-assign-runner-btn"]'), null);
        });
        await act(async () => {
            rootEl.querySelector('[data-testid="rl-cancel-unreachable-open-btn"]').click();
        });
        assert.equal(
            dom.window.document.activeElement?.getAttribute("data-testid"),
            "rl-cancel-unreachable-dismiss",
            "destructive cancel-unreachable dialog must focus Cancel first",
        );
        await act(async () => {
            setControlValue(
                dom.window,
                rootEl.querySelector('[data-testid="rl-cancel-unreachable-reason"]'),
                "Host disconnected >30s",
            );
        });
        await act(async () => {
            rootEl
                .querySelector('[data-testid="rl-cancel-unreachable-dialog"] form')
                .dispatchEvent(new dom.window.Event("submit", { bubbles: true, cancelable: true }));
        });
        await waitForUi(() => {
            assert.equal(state.entriesByProject.proj_team[0].latest_run.status, "Stopped");
        });

        // 5. Append-only Comments: online 201 synced + offline 202 queued_offline + reconnect sync
        await act(async () => {
            rootEl.querySelector('[data-testid="rl-task-tab-comments"]').click();
        });
        await waitForUi(() => {
            assert.ok(rootEl.querySelector('[data-testid="rl-comment-cmt_1"]'));
        });
        await act(async () => {
            setControlValue(
                dom.window,
                rootEl.querySelector('[data-testid="rl-comment-input"]'),
                "Online review note",
            );
        });
        await act(async () => {
            rootEl
                .querySelector('[data-testid="rl-comment-form"]')
                .dispatchEvent(new dom.window.Event("submit", { bubbles: true, cancelable: true }));
        });
        await waitForUi(() => {
            assert.ok(rootEl.textContent.includes("Online review note"));
        });

        // Go offline -> post comment -> 202 queued_offline + offline strip shown
        state.offline = true;
        await act(async () => {
            setControlValue(
                dom.window,
                rootEl.querySelector('[data-testid="rl-comment-input"]'),
                "Offline drafted note",
            );
        });
        await act(async () => {
            rootEl
                .querySelector('[data-testid="rl-comment-form"]')
                .dispatchEvent(new dom.window.Event("submit", { bubbles: true, cancelable: true }));
        });
        await waitForUi(() => {
            assert.ok(rootEl.querySelector('[data-testid="rl-comment-queued-notice"]'));
            assert.ok(rootEl.querySelector('[data-testid="rl-offline-notice"]'));
            assert.ok(rootEl.querySelector('[data-testid="rl-comment-draft-draft_cmt_offline"]'));
        });

        // Reconnect server -> click Sync / Reconnect -> offline strip clears
        state.offline = false;
        await act(async () => {
            rootEl.querySelector('[data-testid="rl-sync-now-btn"]').click();
        });
        await waitForUi(() => {
            assert.equal(rootEl.querySelector('[data-testid="rl-offline-notice"]'), null);
        });

        // 6. Verified CAS Artifacts (200 verified, 404 missing, 500 corrupt_artifact) + RU i18n
        await act(async () => {
            rootEl.querySelector('[data-testid="rl-back-to-board"]').click();
        });
        await waitForUi(() => {
            assert.ok(rootEl.querySelector('[data-testid="rl-card-open-task_accept"]'));
        });
        await act(async () => {
            rootEl.querySelector('[data-testid="rl-card-open-task_accept"]').click();
        });
        await waitForUi(() => {
            assert.ok(rootEl.querySelector('[data-testid="rl-task-tab-diff_artifacts"]'));
        });
        await act(async () => {
            rootEl.querySelector('[data-testid="rl-task-tab-diff_artifacts"]').click();
        });
        await act(async () => {
            rootEl.querySelector('[data-testid="rl-cas-inspect-evidence_bundle"]').click();
            rootEl.querySelector('[data-testid="rl-cas-inspect-verification_log"]').click();
            rootEl.querySelector('[data-testid="rl-cas-inspect-corrupt_diff"]').click();
        });
        await waitForUi(() => {
            assert.match(
                rootEl.querySelector('[data-testid="rl-cas-preview-evidence_bundle"]')
                    ?.textContent ?? "",
                /"verdict":"pass"/,
            );
            assert.ok(rootEl.querySelector('[data-testid="rl-cas-error-corrupt_diff"]'));
            assert.match(
                rootEl.querySelector('[data-testid="rl-cas-artifact-verification_log"]')
                    ?.textContent ?? "",
                /Not synced locally/i,
            );
        });

        // Switch to RU and verify localized team strings
        await act(async () => {
            rootEl.querySelector('[data-testid="rl-lang-ru"]').click();
        });
        await waitForUi(() => {
            assert.ok(rootEl.textContent.includes("SHA-256 проверен"));
            assert.ok(rootEl.textContent.includes("Повреждён · несовпадение SHA-256"));
        });

        // 7. Mobile viewport (< 48em): 5-item mobile nav + mobile bound acceptance bar & confirmation dialog
        mobileMode.value = true;
        await act(async () => root.unmount());
        const mobileRoot = createRoot(rootEl);
        await act(async () => {
            mobileRoot.render(React.createElement(uiPlugin.RunlarkBoardTabBody));
        });
        await waitForUi(() => {
            assert.ok(rootEl.querySelector('[data-testid="rl-mobile-nav-team"]'));
            assert.equal(rootEl.querySelectorAll(".rl-mobile-nav-btn").length, 5);
        });
        // Switch mobile stage to Acceptance and open task_accept
        await act(async () => {
            const stageSelect = rootEl.querySelector(
                '[data-testid="rl-mobile-stage-controls"] select',
            );
            setControlValue(dom.window, stageSelect, "acceptance");
        });
        await waitForUi(() => {
            assert.ok(rootEl.querySelector('[data-testid="rl-card-open-task_accept"]'));
        });
        await act(async () => {
            rootEl.querySelector('[data-testid="rl-card-open-task_accept"]').click();
        });
        await waitForUi(() => {
            assert.ok(rootEl.querySelector('[data-testid="rl-task-tab-acceptance_pr"]'));
        });
        await act(async () => {
            rootEl.querySelector('[data-testid="rl-task-tab-acceptance_pr"]').click();
        });
        await waitForUi(() => {
            assert.ok(rootEl.querySelector('[data-testid="rl-mobile-acceptance-bar"]'));
            assert.ok(rootEl.querySelector('[data-testid="rl-mobile-accept-open"]'));
        });
        await act(async () => {
            rootEl.querySelector('[data-testid="rl-mobile-accept-open"]').click();
        });
        await act(async () => {
            setControlValue(
                dom.window,
                rootEl.querySelector('[data-testid="rl-mobile-accept-comment"]'),
                "Approved on mobile",
            );
        });
        await act(async () => {
            rootEl
                .querySelector('[data-testid="rl-mobile-acceptance-dialog"] form')
                .dispatchEvent(new dom.window.Event("submit", { bubbles: true, cancelable: true }));
        });
        await waitForUi(() => {
            const acceptCall = state.calls.find(
                (c) => c.method === "POST" && c.path === "/api/runlark/tasks/task_accept/accept",
            );
            assert.ok(acceptCall, "mobile accept must POST bound decision");
            assert.equal(acceptCall.body.expected_revision, 5);
            assert.equal(acceptCall.body.plan_version, 2);
            assert.equal(acceptCall.body.head_sha, "2222bbbb");
            assert.equal(acceptCall.body.base_sha, "1111aaaa");
            assert.equal(acceptCall.body.policy_digest, "pol_team_1");
            assert.equal(acceptCall.body.evidence_digest, EVID_DIGEST);
            assert.equal(acceptCall.body.comment, "Approved on mobile");
        });
        await act(async () => mobileRoot.unmount());
    } finally {
        env.restore();
        server.closeAllConnections();
        await new Promise((r) => server.close(r));
    }
});
