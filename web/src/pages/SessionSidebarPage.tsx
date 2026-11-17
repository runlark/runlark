import { useMemo, type ReactNode } from "react";
import type { RunlarkApiClient } from "@/common/runlark-types";
import { SessionCardView, reatomSessionCardModel } from "@/modules/session-card";

export interface SessionSidebarPageProps {
  readonly sessionId: string;
  readonly apiClient: RunlarkApiClient;
  readonly cwd?: string;
  readonly autoStartSync?: boolean;
  readonly pollWaitMs?: number;
}

export function SessionSidebarPage({
  sessionId,
  apiClient,
  cwd,
  autoStartSync = true,
  pollWaitMs,
}: SessionSidebarPageProps): ReactNode {
  const model = useMemo(() => reatomSessionCardModel(apiClient, sessionId), [apiClient, sessionId]);

  return (
    <SessionCardView
      model={model}
      {...(cwd !== undefined ? { defaultCwd: cwd } : {})}
      autoStartSync={autoStartSync}
      {...(pollWaitMs !== undefined ? { pollWaitMs } : {})}
    />
  );
}
