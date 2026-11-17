import { context } from "@reatom/core";
import { reatomContext } from "@reatom/react";
import { useMemo, useState, type ReactNode } from "react";
import { createBoardApiClient } from "@/common/board-http";
import type { BoardApiClient } from "@/common/board-types";
import { getStrings, type Locale } from "@/common/i18n";
import { createRunlarkApiClient } from "@/common/runlark-http";
import type { RunlarkApiClient } from "@/common/runlark-types";
import { ActionButton } from "@/common/ui/ActionButton";
import { WorkbenchTheme } from "@/common/ui/WorkbenchTheme";
import { SessionSidebarPage } from "@/pages/SessionSidebarPage";
import { WorkbenchPage } from "@/pages/WorkbenchPage";

export interface RunlarkSidebarAppProps {
  readonly sessionId: string;
  readonly cwd?: string;
  readonly apiBaseUrl?: string;
  readonly apiMode?: "dsh-proxy" | "daemon";
  readonly apiClient?: RunlarkApiClient;
  readonly boardClient?: BoardApiClient;
  readonly autoStartSync?: boolean;
  readonly pollWaitMs?: number;
  readonly onOpenBoardTab?: () => void;
}

export interface RunlarkWorkbenchAppProps {
  readonly apiBaseUrl?: string;
  readonly apiMode?: "dsh-proxy" | "daemon";
  readonly apiClient?: BoardApiClient;
  readonly initialLocale?: Locale;
  readonly autoStartSync?: boolean;
  readonly pollIntervalMs?: number;
  readonly embedded?: boolean;
}

export function RunlarkSidebarApp({
  sessionId,
  cwd,
  apiBaseUrl = "/api/runlark",
  apiMode = "dsh-proxy",
  apiClient,
  boardClient,
  autoStartSync = true,
  pollWaitMs,
  onOpenBoardTab,
}: RunlarkSidebarAppProps): ReactNode {
  const frame = useMemo(() => context.start(), []);
  const [surface, setSurface] = useState<"session" | "board">("session");
  const client = useMemo(
    () => apiClient ?? createRunlarkApiClient({ baseUrl: apiBaseUrl, mode: apiMode }),
    [apiClient, apiBaseUrl, apiMode],
  );
  const resolvedBoardClient = useMemo(
    () => boardClient ?? createBoardApiClient({ baseUrl: apiBaseUrl, mode: apiMode }),
    [boardClient, apiBaseUrl, apiMode],
  );
  const strings = getStrings("en");

  return (
    <reatomContext.Provider value={frame}>
      <WorkbenchTheme>
        {surface === "board" ? (
          <WorkbenchPage
            apiClient={resolvedBoardClient}
            autoStartSync={autoStartSync}
            embedded
            headerActions={
              <ActionButton
                variant="secondary"
                label={strings.backToSession}
                onClick={() => setSurface("session")}
                testId="rl-switch-to-session"
              />
            }
          />
        ) : (
          <>
            <div className="rl-actions-row">
              <ActionButton
                variant="primary"
                label={strings.openFullBoard}
                onClick={() => {
                  if (onOpenBoardTab) {
                    onOpenBoardTab();
                  } else {
                    setSurface("board");
                  }
                }}
                testId="rl-open-board-btn"
              />
            </div>
            <SessionSidebarPage
              sessionId={sessionId}
              apiClient={client}
              {...(cwd !== undefined ? { cwd } : {})}
              autoStartSync={autoStartSync}
              {...(pollWaitMs !== undefined ? { pollWaitMs } : {})}
            />
          </>
        )}
      </WorkbenchTheme>
    </reatomContext.Provider>
  );
}

export function RunlarkWorkbenchApp({
  apiBaseUrl = "/api/runlark",
  apiMode = "dsh-proxy",
  apiClient,
  initialLocale = "en",
  autoStartSync = true,
  pollIntervalMs,
  embedded = false,
}: RunlarkWorkbenchAppProps): ReactNode {
  const frame = useMemo(() => context.start(), []);
  const client = useMemo(
    () => apiClient ?? createBoardApiClient({ baseUrl: apiBaseUrl, mode: apiMode }),
    [apiClient, apiBaseUrl, apiMode],
  );

  return (
    <reatomContext.Provider value={frame}>
      <WorkbenchTheme>
        <WorkbenchPage
          apiClient={client}
          initialLocale={initialLocale}
          autoStartSync={autoStartSync}
          {...(pollIntervalMs !== undefined ? { pollIntervalMs } : {})}
          embedded={embedded}
        />
      </WorkbenchTheme>
    </reatomContext.Provider>
  );
}
