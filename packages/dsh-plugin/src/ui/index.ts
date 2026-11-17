import type { Context as ClientContext } from "@deepseek-ai/cordis";
import type {} from "@deepseek-ai/dsh-client-ui-renderer/client";
import type {} from "@deepseek-ai/dsh-client-ui-session/client";
import type {} from "@deepseek-ai/dsh-client-ui-sidebar-right/client";
import {
  RUNLARK_TAB_ID,
  RUNLARK_TAB_KIND,
  RunlarkMarkGlyph,
  RunlarkTabBody,
  RunlarkTabTitle,
} from "./slots";

export { RUNLARK_TAB_ID, RUNLARK_TAB_KIND, RunlarkTabBody, RunlarkTabTitle };

export const inject = ["sidebarRightTabs", "sidebarRight", "slots", "uiSession"];

async function checkPendingApprovalAndOpenSidebar(
  ctx: ClientContext,
  sessionId: string,
  signal: AbortSignal,
): Promise<void> {
  try {
    const res = await fetch(`/api/runlark/sessions/${encodeURIComponent(sessionId)}`, { signal });
    if (!res.ok || signal.aborted) {
      return;
    }
    const data = (await res.json()) as {
      active_run_status?: string | null;
      pending_question?: { status?: string } | null;
    };
    if (
      !signal.aborted &&
      (data.active_run_status === "waiting_for_approval" ||
        data.pending_question?.status === "pending")
    ) {
      ctx.sidebarRight.openTab(RUNLARK_TAB_KIND);
    }
  } catch {
    // Daemon may be offline or session unmanaged; sidebar remains manually openable via guide.
  }
}

export function apply(ctx: ClientContext): void {
  ctx.effect(
    () =>
      ctx.sidebarRightTabs.register({
        id: RUNLARK_TAB_ID,
        kind: RUNLARK_TAB_KIND,
        priority: "extension",
        keepMounted: true,
        title: () => "Runlark",
        guide: [
          {
            id: "runlark-session",
            order: 15,
            title: () => "Runlark Run",
            description: () => "Session state, live events, and pending approvals",
            icon: RunlarkMarkGlyph,
          },
        ],
      }),
    "runlark-ui: right-sidebar tab type",
  );

  ctx.effect(
    () =>
      ctx.slots.inject("sidebar.right.pane.tab", () =>
        ctx.slots.register(
          {
            name: "sidebar.right.pane.tab",
            key: RUNLARK_TAB_ID,
          },
          RunlarkTabBody,
        ),
      ),
    "runlark-ui: right-sidebar tab body",
  );

  ctx.effect(
    () =>
      ctx.slots.inject("sidebar.right.pane.tab.title", () =>
        ctx.slots.register(
          {
            name: "sidebar.right.pane.tab.title",
            key: RUNLARK_TAB_ID,
          },
          RunlarkTabTitle,
        ),
      ),
    "runlark-ui: right-sidebar tab title",
  );

  ctx.effect(() => {
    let activeController: AbortController | undefined;
    const current = ctx.uiSession.adapter.current;

    const inspectCurrent = (): void => {
      activeController?.abort();
      const key = current.getSnapshot().key;
      if (typeof key !== "string" || !key) {
        return;
      }
      activeController = new AbortController();
      void checkPendingApprovalAndOpenSidebar(ctx, key, activeController.signal);
    };

    const unsubscribe = current.subscribe(inspectCurrent);
    inspectCurrent();

    return () => {
      unsubscribe();
      activeController?.abort();
    };
  }, "runlark-ui: restore WaitingForApproval sidebar surface");
}
