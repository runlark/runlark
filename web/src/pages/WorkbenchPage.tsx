import { useMemo, type ReactNode } from "react";
import type { BoardApiClient } from "@/common/board-types";
import type { Locale } from "@/common/i18n";
import { TaskFlowWorkspaceView, reatomTaskFlowModel } from "@/modules/task-flow";

export interface WorkbenchPageProps {
  readonly apiClient: BoardApiClient;
  readonly initialLocale?: Locale;
  readonly autoStartSync?: boolean;
  readonly pollIntervalMs?: number;
  readonly embedded?: boolean;
  readonly headerActions?: ReactNode;
}

export function WorkbenchPage({
  apiClient,
  initialLocale = "en",
  autoStartSync = true,
  pollIntervalMs,
  embedded = false,
  headerActions,
}: WorkbenchPageProps): ReactNode {
  const model = useMemo(
    () => reatomTaskFlowModel(apiClient, initialLocale),
    [apiClient, initialLocale],
  );

  return (
    <TaskFlowWorkspaceView
      model={model}
      autoStartSync={autoStartSync}
      {...(pollIntervalMs !== undefined ? { pollIntervalMs } : {})}
      embedded={embedded}
      {...(headerActions !== undefined ? { headerActions } : {})}
    />
  );
}
