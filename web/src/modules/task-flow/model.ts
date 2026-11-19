import { action, atom, computed, sleep, wrap } from "@reatom/core";
import {
  TASK_STAGES,
  hasOwnPendingDecision,
  taskStageLabel,
  type BoardApiClient,
  type BoardResponse,
  type BoardTaskEntry,
  type ChatDispatchResult,
  type InstalledPlugin,
  type ProjectSummary,
  type RunDiffResponse,
  type RunHistoryResponse,
  type TaskKind,
  type TaskStage,
} from "@/common/board-types";
import { getStrings, type Locale } from "@/common/i18n";
import type { DaemonEventItem } from "@/common/runlark-types";
import type { WorkbenchSection } from "@/common/ui/WorkbenchShell";

export type TaskDetailTab =
  | "workflow"
  | "run_budget"
  | "diff_artifacts"
  | "acceptance_pr"
  | "comments"
  | "events_chat";

export interface TaskFlowModel {
  readonly locale: ReturnType<typeof atom<Locale>>;
  readonly section: ReturnType<typeof atom<WorkbenchSection>>;
  readonly selectedProjectId: ReturnType<typeof atom<string | null>>;
  readonly selectedTaskId: ReturnType<typeof atom<string | null>>;
  readonly activeTaskTab: ReturnType<typeof atom<TaskDetailTab>>;
  readonly mobileStage: ReturnType<typeof atom<TaskStage>>;
  readonly searchQuery: ReturnType<typeof atom<string>>;
  readonly kindFilter: ReturnType<typeof atom<"all" | TaskKind>>;
  readonly decisionsOnly: ReturnType<typeof atom<boolean>>;
  readonly board: ReturnType<typeof atom<BoardResponse | null>>;
  readonly projects: ReturnType<typeof atom<readonly ProjectSummary[]>>;
  readonly plugins: ReturnType<typeof atom<readonly InstalledPlugin[]>>;
  readonly selectedTaskDetail: ReturnType<typeof atom<BoardTaskEntry | null>>;
  readonly selectedRunHistory: ReturnType<typeof atom<RunHistoryResponse | null>>;
  readonly selectedRunDiff: ReturnType<typeof atom<RunDiffResponse | null>>;
  readonly chatLog: ReturnType<typeof atom<readonly ChatDispatchResult[]>>;
  readonly loading: ReturnType<typeof atom<boolean>>;
  readonly actionPending: ReturnType<typeof atom<string | null>>;
  readonly actionError: ReturnType<typeof atom<string | null>>;
  readonly announcement: ReturnType<typeof atom<string>>;
  readonly prUnknownReason: ReturnType<typeof atom<string | null>>;
  readonly createDialogKind: ReturnType<typeof atom<TaskKind | null>>;
  readonly filteredColumns: ReturnType<
    typeof computed<Readonly<Record<TaskStage, readonly BoardTaskEntry[]>>>
  >;
  readonly decisionEntries: ReturnType<typeof computed<readonly BoardTaskEntry[]>>;
  readonly selectedRunEvents: ReturnType<typeof computed<readonly DaemonEventItem[]>>;
  readonly setLocale: (next: Locale) => void;
  readonly navigateTo: (next: WorkbenchSection) => void;
  readonly selectProject: (projectId: string) => Promise<void>;
  readonly openTask: (taskId: string) => Promise<void>;
  readonly closeTask: () => void;
  readonly openCreateDialog: (kind: TaskKind) => void;
  readonly closeCreateDialog: () => void;
  readonly refreshAll: () => Promise<void>;
  readonly createProject: (input: {
    readonly name: string;
    readonly repo_root: string;
    readonly default_branch?: string;
  }) => Promise<void>;
  readonly createTask: (input: {
    readonly title: string;
    readonly description: string;
    readonly kind: TaskKind;
  }) => Promise<void>;
  readonly moveTask: (taskId: string, stage: TaskStage, expectedRevision: number) => Promise<void>;
  readonly clarifyTask: (taskId: string, clarification: string) => Promise<void>;
  readonly savePlan: (
    taskId: string,
    planText: string,
    approve: boolean,
    expectedRevision: number,
  ) => Promise<void>;
  readonly startTask: (taskId: string) => Promise<void>;
  readonly stopRun: (sessionId: string, runId: string, expectedRevision: number) => Promise<void>;
  readonly resumeSession: (
    sessionId: string,
    runId: string,
    expectedRevision: number,
  ) => Promise<void>;
  readonly verifyRun: (runId: string, taskId: string) => Promise<void>;
  readonly answerQuestion: (
    questionId: string,
    answer: string,
    sessionId: string,
    expectedRevision: number,
  ) => Promise<void>;
  readonly acceptTask: (
    taskId: string,
    input: {
      readonly comment?: string;
      readonly expected_revision?: number;
      readonly plan_version?: number;
      readonly head_sha?: string;
      readonly base_sha?: string;
      readonly policy_digest?: string;
      readonly evidence_digest?: string;
    },
  ) => Promise<void>;
  readonly rejectTask: (taskId: string, reason: string) => Promise<void>;
  readonly createPr: (
    taskId: string,
    input: { readonly title?: string; readonly base_branch?: string; readonly draft: boolean },
  ) => Promise<void>;
  readonly dispatchChat: (input: string) => Promise<void>;
  readonly installPlugin: (path: string) => Promise<void>;
  readonly startBoardSync: (pollIntervalMs?: number) => () => void;
}

function errMsg(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export function reatomTaskFlowModel(
  api: BoardApiClient,
  initialLocale: Locale = "en",
  onMutationError?: (error: unknown, key: string) => void,
): TaskFlowModel {
  const prefix = "taskFlow";
  const locale = atom<Locale>(initialLocale, `${prefix}.locale`);
  const section = atom<WorkbenchSection>("board", `${prefix}.section`);
  const selectedProjectId = atom<string | null>(null, `${prefix}.selectedProjectId`);
  const selectedTaskId = atom<string | null>(null, `${prefix}.selectedTaskId`);
  const activeTaskTab = atom<TaskDetailTab>("workflow", `${prefix}.activeTaskTab`);
  const mobileStage = atom<TaskStage>("inbox", `${prefix}.mobileStage`);
  const searchQuery = atom<string>("", `${prefix}.searchQuery`);
  const kindFilter = atom<"all" | TaskKind>("all", `${prefix}.kindFilter`);
  const decisionsOnly = atom<boolean>(false, `${prefix}.decisionsOnly`);
  const board = atom<BoardResponse | null>(null, `${prefix}.board`);
  const projects = atom<readonly ProjectSummary[]>([], `${prefix}.projects`);
  const plugins = atom<readonly InstalledPlugin[]>([], `${prefix}.plugins`);
  const selectedTaskDetail = atom<BoardTaskEntry | null>(null, `${prefix}.selectedTaskDetail`);
  const selectedRunHistory = atom<RunHistoryResponse | null>(null, `${prefix}.selectedRunHistory`);
  const selectedRunDiff = atom<RunDiffResponse | null>(null, `${prefix}.selectedRunDiff`);
  const chatLog = atom<readonly ChatDispatchResult[]>([], `${prefix}.chatLog`);
  const loading = atom<boolean>(true, `${prefix}.loading`);
  const actionPending = atom<string | null>(null, `${prefix}.actionPending`);
  const actionError = atom<string | null>(null, `${prefix}.actionError`);
  const announcement = atom<string>("", `${prefix}.announcement`);
  const prUnknownReason = atom<string | null>(null, `${prefix}.prUnknownReason`);
  const createDialogKind = atom<TaskKind | null>(null, `${prefix}.createDialogKind`);

  const decisionEntries = computed<readonly BoardTaskEntry[]>(() => {
    const b = board();
    if (!b) return [];
    return b.tasks.filter(hasOwnPendingDecision);
  }, `${prefix}.decisionEntries`);

  const filteredColumns = computed<Readonly<Record<TaskStage, readonly BoardTaskEntry[]>>>(() => {
    const b = board();
    const empty: Record<TaskStage, readonly BoardTaskEntry[]> = {
      inbox: [],
      ready: [],
      in_progress: [],
      acceptance: [],
      done: [],
    };
    if (!b) return empty;
    const q = searchQuery().trim().toLowerCase();
    const kf = kindFilter();
    const decOnly = decisionsOnly() || section() === "decisions";

    const matches = (entry: BoardTaskEntry): boolean => {
      if (kf !== "all" && entry.task.kind !== kf) return false;
      if (decOnly && !hasOwnPendingDecision(entry)) return false;
      if (!q) return true;
      return (
        entry.task.title.toLowerCase().includes(q) ||
        entry.task.id.toLowerCase().includes(q) ||
        entry.task.workflow_stage.toLowerCase().includes(q)
      );
    };

    const result: Record<TaskStage, BoardTaskEntry[]> = {
      inbox: [],
      ready: [],
      in_progress: [],
      acceptance: [],
      done: [],
    };
    for (const stage of TASK_STAGES) {
      result[stage] = b.columns[stage].filter(matches);
    }
    return result;
  }, `${prefix}.filteredColumns`);

  const selectedRunEvents = computed<readonly DaemonEventItem[]>(
    () => selectedRunHistory()?.events ?? [],
    `${prefix}.selectedRunEvents`,
  );

  let detailAbort: AbortController | null = null;

  const loadTaskDetail = action(async (taskId: string, signal?: AbortSignal) => {
    const detail = await wrap(api.getTask(taskId, signal));
    if (signal?.aborted || selectedTaskId() !== taskId) return;
    selectedTaskDetail.set(detail);
    const runId = detail.latest_run?.id ?? detail.task.latest_run_id ?? detail.task.active_run_id;
    if (!runId) {
      selectedRunHistory.set(null);
      selectedRunDiff.set(null);
      return;
    }
    const [hist, diff] = await wrap(
      Promise.all([
        api.getRunHistory(runId, signal).catch(() => null),
        api.getRunDiff(runId, signal).catch(() => null),
      ]),
    );
    if (signal?.aborted || selectedTaskId() !== taskId) return;
    selectedRunHistory.set(hist);
    selectedRunDiff.set(diff);
  }, `${prefix}.loadTaskDetail`);

  const syncOnce = action(async (signal?: AbortSignal) => {
    const [loadedProjects, loadedPlugins] = await wrap(
      Promise.all([
        api.listProjects(signal),
        api.listPlugins(signal).catch(() => [] as readonly InstalledPlugin[]),
      ]),
    );
    if (signal?.aborted) return;
    projects.set(loadedProjects);
    plugins.set(loadedPlugins);

    const pid = selectedProjectId() ?? loadedProjects[0]?.project_id;
    if (pid && !selectedProjectId()) {
      selectedProjectId.set(pid);
    }
    const loadedBoard = await wrap(api.getBoard(pid, signal));
    if (signal?.aborted) return;
    board.set(loadedBoard);

    const openTaskId = selectedTaskId();
    if (openTaskId) {
      await wrap(loadTaskDetail(openTaskId, signal));
    }
    loading.set(false);
  }, `${prefix}.syncOnce`);

  const refreshAll = action(async () => {
    try {
      await wrap(syncOnce());
      actionError.set(null);
    } catch (error: unknown) {
      loading.set(false);
      actionError.set(errMsg(error));
    }
  }, `${prefix}.refreshAll`);

  const runMutation = async (key: string, fn: () => Promise<void>): Promise<void> => {
    actionPending.set(key);
    actionError.set(null);
    try {
      await wrap(fn());
      await wrap(syncOnce());
    } catch (error: unknown) {
      actionError.set(errMsg(error));
      onMutationError?.(error, key);
    } finally {
      actionPending.set(null);
    }
  };

  const setLocale = action((next: Locale) => {
    locale.set(next);
  }, `${prefix}.setLocale`);

  const navigateTo = action((next: WorkbenchSection) => {
    detailAbort?.abort();
    detailAbort = null;
    section.set(next);
    selectedTaskId.set(null);
    selectedTaskDetail.set(null);
    if (next === "decisions") {
      decisionsOnly.set(true);
    } else if (next === "board") {
      decisionsOnly.set(false);
    }
  }, `${prefix}.navigateTo`);

  const selectProject = action(async (projectId: string) => {
    selectedProjectId.set(projectId);
    selectedTaskId.set(null);
    await wrap(refreshAll());
  }, `${prefix}.selectProject`);

  const openTask = action(async (taskId: string) => {
    detailAbort?.abort();
    const controller = new AbortController();
    detailAbort = controller;
    selectedTaskId.set(taskId);
    prUnknownReason.set(null);
    actionError.set(null);
    try {
      await wrap(loadTaskDetail(taskId, controller.signal));
    } catch (error: unknown) {
      if (!controller.signal.aborted) {
        actionError.set(errMsg(error));
      }
    }
  }, `${prefix}.openTask`);

  const closeTask = action(() => {
    detailAbort?.abort();
    detailAbort = null;
    selectedTaskId.set(null);
    selectedTaskDetail.set(null);
    selectedRunHistory.set(null);
    selectedRunDiff.set(null);
  }, `${prefix}.closeTask`);

  const openCreateDialog = action((kind: TaskKind) => {
    actionError.set(null);
    createDialogKind.set(kind);
  }, `${prefix}.openCreateDialog`);

  const closeCreateDialog = action(() => {
    createDialogKind.set(null);
  }, `${prefix}.closeCreateDialog`);

  const createProject = action(
    (input: {
      readonly name: string;
      readonly repo_root: string;
      readonly default_branch?: string;
    }) =>
      runMutation("createProject", async () => {
        const created = await wrap(api.createProject(input));
        selectedProjectId.set(created.project_id);
      }),
    `${prefix}.createProject`,
  );

  const createTask = action(
    (input: { readonly title: string; readonly description: string; readonly kind: TaskKind }) =>
      runMutation("createTask", async () => {
        const pid = selectedProjectId() ?? undefined;
        await wrap(api.createTask({ ...(pid ? { project_id: pid } : {}), ...input }));
        createDialogKind.set(null);
      }),
    `${prefix}.createTask`,
  );

  const moveTask = action(
    (taskId: string, stage: TaskStage, expectedRevision: number) =>
      runMutation("moveTask", async () => {
        const moved = await wrap(api.moveTask(taskId, stage, expectedRevision));
        const strings = getStrings(locale());
        announcement.set(strings.movedAnnouncement(moved.title, taskStageLabel(stage, strings)));
      }),
    `${prefix}.moveTask`,
  );

  const clarifyTask = action(
    (taskId: string, clarification: string) =>
      runMutation("clarify", async () => {
        await wrap(api.clarifyTask(taskId, clarification, true));
      }),
    `${prefix}.clarifyTask`,
  );

  const savePlan = action(
    (taskId: string, planText: string, approve: boolean, expectedRevision: number) =>
      runMutation("plan", async () => {
        await wrap(
          api.planTask(taskId, {
            plan_text: planText,
            approve,
            expected_revision: expectedRevision,
          }),
        );
      }),
    `${prefix}.savePlan`,
  );

  const startTask = action(
    (taskId: string) =>
      runMutation("start", async () => {
        await wrap(api.startTask(taskId));
      }),
    `${prefix}.startTask`,
  );

  const stopRun = action(
    (sessionId: string, runId: string, expectedRevision: number) =>
      runMutation("stop", async () => {
        await wrap(api.stopRun({ sessionId, runId, expectedRevision }));
      }),
    `${prefix}.stopRun`,
  );

  const resumeSession = action(
    (sessionId: string, runId: string, expectedRevision: number) =>
      runMutation("resume", async () => {
        await wrap(api.resumeSession({ sessionId, runId, expectedRevision }));
      }),
    `${prefix}.resumeSession`,
  );

  const verifyRun = action(
    (runId: string, taskId: string) =>
      runMutation("verify", async () => {
        await wrap(api.verifyRun(runId, taskId));
      }),
    `${prefix}.verifyRun`,
  );

  const answerQuestion = action(
    (questionId: string, answer: string, sessionId: string, expectedRevision: number) =>
      runMutation("answer", async () => {
        await wrap(api.answerQuestion({ sessionId, questionId, answer, expectedRevision }));
      }),
    `${prefix}.answerQuestion`,
  );

  const acceptTask = action(
    (
      taskId: string,
      input: {
        readonly comment?: string;
        readonly expected_revision?: number;
        readonly plan_version?: number;
        readonly head_sha?: string;
        readonly base_sha?: string;
        readonly policy_digest?: string;
        readonly evidence_digest?: string;
      },
    ) =>
      runMutation("accept", async () => {
        await wrap(api.acceptTask(taskId, input));
      }),
    `${prefix}.acceptTask`,
  );

  const rejectTask = action(
    (taskId: string, reason: string) =>
      runMutation("reject", async () => {
        await wrap(api.rejectTask(taskId, { reason }));
      }),
    `${prefix}.rejectTask`,
  );

  const createPr = action(
    (
      taskId: string,
      input: { readonly title?: string; readonly base_branch?: string; readonly draft: boolean },
    ) =>
      runMutation("pr", async () => {
        const res = await wrap(api.createTaskPr(taskId, input));
        if (res.outcome === "unknown") {
          prUnknownReason.set(res.reason ?? "unknown");
        } else {
          prUnknownReason.set(null);
        }
      }),
    `${prefix}.createPr`,
  );

  const dispatchChat = action(
    (input: string) =>
      runMutation("chat", async () => {
        const detail = selectedTaskDetail();
        const pid = selectedProjectId() ?? undefined;
        const sid = detail?.latest_run?.session_id ?? detail?.task.active_session_id ?? undefined;
        const rid = detail?.latest_run?.id ?? undefined;
        const res = await wrap(
          api.dispatchChat({
            input,
            ...(pid ? { project_id: pid } : {}),
            ...(sid ? { session_id: sid } : {}),
            ...(rid ? { run_id: rid } : {}),
          }),
        );
        chatLog.set([res, ...chatLog().slice(0, 19)]);
      }),
    `${prefix}.dispatchChat`,
  );

  const installPlugin = action(
    (path: string) =>
      runMutation("plugin", async () => {
        await wrap(api.installPlugin(path));
      }),
    `${prefix}.installPlugin`,
  );

  const startBoardSync = (pollIntervalMs = 2500): (() => void) => {
    const controller = new AbortController();
    const { signal } = controller;

    const runLoop = action(async () => {
      while (!signal.aborted) {
        try {
          await wrap(syncOnce(signal));
        } catch (error: unknown) {
          if (signal.aborted) break;
          loading.set(false);
          actionError.set(errMsg(error));
        }
        try {
          await wrap(sleep(pollIntervalMs));
        } catch {
          break;
        }
      }
    }, `${prefix}.boardLoop`);

    void runLoop();
    return () => controller.abort();
  };

  return {
    locale,
    section,
    selectedProjectId,
    selectedTaskId,
    activeTaskTab,
    mobileStage,
    searchQuery,
    kindFilter,
    decisionsOnly,
    board,
    projects,
    plugins,
    selectedTaskDetail,
    selectedRunHistory,
    selectedRunDiff,
    chatLog,
    loading,
    actionPending,
    actionError,
    announcement,
    prUnknownReason,
    createDialogKind,
    filteredColumns,
    decisionEntries,
    selectedRunEvents,
    setLocale,
    navigateTo,
    selectProject,
    openTask,
    closeTask,
    openCreateDialog,
    closeCreateDialog,
    refreshAll,
    createProject,
    createTask,
    moveTask,
    clarifyTask,
    savePlan,
    startTask,
    stopRun,
    resumeSession,
    verifyRun,
    answerQuestion,
    acceptTask,
    rejectTask,
    createPr,
    dispatchChat,
    installPlugin,
    startBoardSync,
  };
}
