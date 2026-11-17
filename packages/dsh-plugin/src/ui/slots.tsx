import type {} from "@deepseek-ai/dsh-client-ui-session/client";
import type {} from "@deepseek-ai/dsh-client-ui-sidebar-right/client";
import type { PropsRuntime } from "@deepseek-ai/dsh-client-ui-slots";
import type { ReactNode } from "react";
import { RunlarkSidebarApp } from "../../../../web/src/app/index";

export const RUNLARK_TAB_ID = "@runlark/dsh-plugin";
export const RUNLARK_TAB_KIND = "runlark";

export function RunlarkMarkGlyph(): ReactNode {
  return (
    <svg
      width={15}
      height={15}
      viewBox="0 0 16 16"
      fill="none"
      aria-hidden="true"
      focusable="false"
    >
      <rect x="2" y="2.5" width="12" height="11" rx="2" stroke="currentColor" strokeWidth="1.4" />
      <path d="M5 6.2H11M5 9.8H9.2" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
    </svg>
  );
}

export function RunlarkTabTitle({
  useTabInfo,
}: PropsRuntime<"sidebar.right.pane.tab.title">): ReactNode {
  const { tab } = useTabInfo();
  return (
    <>
      <RunlarkMarkGlyph />
      <span>{tab.title}</span>
    </>
  );
}

export function RunlarkTabBody({
  sessionId,
  useSessions,
}: PropsRuntime<"sidebar.right.pane.tab">): ReactNode {
  const cwd = useSessions((sessions) => sessions.byId[sessionId]?.cwd);

  return (
    <RunlarkSidebarApp
      sessionId={sessionId}
      {...(cwd !== undefined ? { cwd } : {})}
      apiBaseUrl="/api/runlark"
      apiMode="dsh-proxy"
    />
  );
}
