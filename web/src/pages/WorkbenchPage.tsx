import { useMemo, type ReactNode } from "react";
import type { BoardApiClient } from "@/common/board-types";
import type { Locale } from "@/common/i18n";
import type { TeamApiClient } from "@/common/team-types";
import { TaskFlowWorkspaceView, reatomTaskFlowModel } from "@/modules/task-flow";
import { reatomTeamCollabModel } from "@/modules/team-collab";

export interface WorkbenchPageProps {
  readonly apiClient: BoardApiClient;
  readonly teamClient: TeamApiClient;
  readonly initialLocale?: Locale;
  readonly autoStartSync?: boolean;
  readonly pollIntervalMs?: number;
  readonly embedded?: boolean;
  readonly headerActions?: ReactNode;
}

export function WorkbenchPage({
  apiClient,
  teamClient,
  initialLocale = "en",
  autoStartSync = true,
  pollIntervalMs,
  embedded = false,
  headerActions,
}: WorkbenchPageProps): ReactNode {
  const { model, teamModel } = useMemo(() => {
    let taskModelRef: ReturnType<typeof reatomTaskFlowModel> | null = null;
    const team = reatomTeamCollabModel(teamClient, {
      onBoardInvalidated: async () => {
        if (taskModelRef) {
          await taskModelRef.refreshAll();
        }
      },
    });
    const taskModel = reatomTaskFlowModel(apiClient, initialLocale, (err, key) => {
      team.recordApiFailure(err, key);
    });
    taskModelRef = taskModel;
    return { model: taskModel, teamModel: team };
  }, [apiClient, teamClient, initialLocale]);

  return (
    <TaskFlowWorkspaceView
      model={model}
      teamModel={teamModel}
      autoStartSync={autoStartSync}
      {...(pollIntervalMs !== undefined ? { pollIntervalMs } : {})}
      embedded={embedded}
      {...(headerActions !== undefined ? { headerActions } : {})}
    />
  );
}
