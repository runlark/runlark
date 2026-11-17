import { context } from "@reatom/core";
import { reatomContext } from "@reatom/react";
import { useMemo, type ReactNode } from "react";
import { createRunlarkApiClient } from "@/common/runlark-http";
import type { RunlarkApiClient } from "@/common/runlark-types";
import { WorkbenchTheme } from "@/common/ui/WorkbenchTheme";
import { SessionSidebarPage } from "@/pages/SessionSidebarPage";

export interface RunlarkSidebarAppProps {
  readonly sessionId: string;
  readonly cwd?: string;
  readonly apiBaseUrl?: string;
  readonly apiMode?: "dsh-proxy" | "daemon";
  readonly apiClient?: RunlarkApiClient;
  readonly autoStartSync?: boolean;
  readonly pollWaitMs?: number;
}

export function RunlarkSidebarApp({
  sessionId,
  cwd,
  apiBaseUrl = "/api/runlark",
  apiMode = "dsh-proxy",
  apiClient,
  autoStartSync = true,
  pollWaitMs,
}: RunlarkSidebarAppProps): ReactNode {
  const frame = useMemo(() => context.start(), []);
  const client = useMemo(
    () => apiClient ?? createRunlarkApiClient({ baseUrl: apiBaseUrl, mode: apiMode }),
    [apiClient, apiBaseUrl, apiMode],
  );

  return (
    <reatomContext.Provider value={frame}>
      <WorkbenchTheme>
        <SessionSidebarPage
          sessionId={sessionId}
          apiClient={client}
          {...(cwd !== undefined ? { cwd } : {})}
          autoStartSync={autoStartSync}
          {...(pollWaitMs !== undefined ? { pollWaitMs } : {})}
        />
      </WorkbenchTheme>
    </reatomContext.Provider>
  );
}
