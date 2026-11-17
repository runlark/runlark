import type { Locale, RunlarkStrings } from "./i18n";
import type { DaemonEventItem, DaemonQuestion, RunStatus } from "./runlark-types";

export type TaskStage = "inbox" | "ready" | "in_progress" | "acceptance" | "done";
export type TaskKind = "standard" | "quick_fix";
export type WorkflowStage =
  | "clarification"
  | "plan"
  | "implementation"
  | "verification"
  | "human_acceptance";

export const TASK_STAGES: readonly TaskStage[] = [
  "inbox",
  "ready",
  "in_progress",
  "acceptance",
  "done",
];

export interface BudgetPolicy {
  readonly max_attempts: number;
  readonly max_duration_ms: number;
  readonly max_spend_micro_usd: number;
  readonly allow_unknown_cost_repair: boolean;
}

export interface CostSummary {
  readonly known_micro_usd: number;
  readonly held_micro_usd: number;
  readonly unknown_attempts: number;
}

export interface ProjectSummary {
  readonly project_id: string;
  readonly name: string;
  readonly repo_root: string;
  readonly default_branch: string;
  readonly max_concurrent_runs: number;
  readonly policy_digest: string;
  readonly budget_policy: BudgetPolicy;
  readonly created_at: number;
  readonly updated_at: number;
}

export interface TaskRecord {
  readonly id: string;
  readonly project_id: string;
  readonly title: string;
  readonly description: string;
  readonly kind: TaskKind;
  readonly stage: TaskStage;
  readonly workflow_stage: WorkflowStage;
  readonly plan_version: number;
  readonly plan_text?: string | null;
  readonly clarification_notes?: string | null;
  readonly active_run_id?: string | null;
  readonly latest_run_id?: string | null;
  readonly active_session_id?: string | null;
  readonly base_ref?: string | null;
  readonly head_ref?: string | null;
  readonly pr_url?: string | null;
  readonly revision: number;
  readonly created_at: number;
  readonly updated_at: number;
}

export interface RunRecord {
  readonly id: string;
  readonly session_id: string;
  readonly status: string;
  readonly worktree_path: string;
  readonly revision: number;
  readonly task_id?: string | null;
  readonly project_id?: string | null;
  readonly workflow_stage: WorkflowStage;
  readonly task_kind: TaskKind;
  readonly workflow_digest?: string | null;
  readonly base_sha?: string | null;
  readonly head_sha?: string | null;
  readonly plan_version: number;
  readonly policy_digest?: string | null;
  readonly budget_policy: BudgetPolicy;
  readonly cost_summary: CostSummary;
  readonly repair_attempts: number;
  readonly pr_url?: string | null;
  readonly created_at: number;
  readonly updated_at: number;
}

export interface AttemptRecord {
  readonly id: string;
  readonly run_id: string;
  readonly attempt_number: number;
  readonly workflow_stage: WorkflowStage;
  readonly outcome: { readonly status: string; readonly error?: string; readonly reason?: string };
  readonly cost_micro_usd: number | null;
  readonly unknown_cost_reason?: string | null;
  readonly repair_reason?: string | null;
  readonly started_at: number;
  readonly finished_at?: number | null;
}

export interface DecisionRecord {
  readonly id: string;
  readonly task_id: string;
  readonly run_id: string;
  readonly kind: "accept" | "request_changes";
  readonly actor: string;
  readonly plan_version: number;
  readonly head_sha: string;
  readonly base_sha: string;
  readonly policy_digest: string;
  readonly evidence_digest: string;
  readonly comment?: string | null;
  readonly created_at: number;
}

export interface ArtifactItem {
  readonly digest_sha256: string;
  readonly kind: string;
  readonly media_type: string;
  readonly size_bytes: number;
  readonly label: string;
  readonly created_at: number;
}

export interface BoardTaskEntry {
  readonly task: TaskRecord;
  readonly latest_run: RunRecord | null;
  readonly session_revision?: number | null;
  readonly attempts: readonly AttemptRecord[];
  readonly cost_summary: CostSummary;
  readonly decisions: readonly DecisionRecord[];
  readonly artifacts: readonly ArtifactItem[];
  readonly pending_questions: readonly DaemonQuestion[];
}

export interface BoardColumns {
  readonly inbox: readonly BoardTaskEntry[];
  readonly ready: readonly BoardTaskEntry[];
  readonly in_progress: readonly BoardTaskEntry[];
  readonly acceptance: readonly BoardTaskEntry[];
  readonly done: readonly BoardTaskEntry[];
}

export interface SlotSummary {
  readonly max: number;
  readonly used: number;
  readonly available: number;
}

export interface BoardResponse {
  readonly project: ProjectSummary | null;
  readonly columns: BoardColumns;
  readonly tasks: readonly BoardTaskEntry[];
  readonly active_runs: readonly RunRecord[];
  readonly queued_runs: readonly RunRecord[];
  readonly attempts: readonly AttemptRecord[];
  readonly cost_summary: CostSummary;
  readonly decisions: readonly DecisionRecord[];
  readonly pending_questions: readonly DaemonQuestion[];
  readonly slots: SlotSummary;
}

export interface RunDiffResponse {
  readonly run_id: string;
  readonly base_sha: string;
  readonly head_sha: string;
  readonly branch_name: string;
  readonly changed_files: readonly string[];
  readonly diff: string;
  readonly diff_bytes: number;
  readonly digest_sha256: string;
}

export interface RunHistoryResponse {
  readonly run: RunRecord;
  readonly attempts: readonly AttemptRecord[];
  readonly cost_summary: CostSummary;
  readonly decisions: readonly DecisionRecord[];
  readonly artifacts: readonly ArtifactItem[];
  readonly events: readonly DaemonEventItem[];
  readonly pending_questions: readonly DaemonQuestion[];
}

export interface InstalledPlugin {
  readonly name: string;
  readonly version: string;
  readonly package_digest: string;
  readonly stages: readonly string[];
}

export interface ChatDispatchResult {
  readonly command: string;
  readonly summary: string;
  readonly data: unknown;
}

export interface CreatePrOutcome {
  readonly outcome: "created" | "unknown";
  readonly command_id: string;
  readonly run_id: string;
  readonly branch?: string;
  readonly pr_url?: string;
  readonly receipt_digest?: string;
  readonly replayed?: boolean;
  readonly reason?: string;
}

export interface BoardApiClient {
  getBoard(projectId?: string, signal?: AbortSignal): Promise<BoardResponse>;
  listProjects(signal?: AbortSignal): Promise<readonly ProjectSummary[]>;
  createProject(input: {
    readonly name: string;
    readonly repo_root: string;
    readonly default_branch?: string;
  }): Promise<ProjectSummary>;
  getTask(taskId: string, signal?: AbortSignal): Promise<BoardTaskEntry>;
  createTask(input: {
    readonly project_id?: string;
    readonly title: string;
    readonly description: string;
    readonly kind: TaskKind;
  }): Promise<TaskRecord>;
  moveTask(taskId: string, stage: TaskStage, expectedRevision: number): Promise<TaskRecord>;
  clarifyTask(
    taskId: string,
    clarification: string,
    advanceToPlan?: boolean,
  ): Promise<{ readonly task: TaskRecord; readonly workflow_stage: WorkflowStage }>;
  planTask(
    taskId: string,
    input: {
      readonly plan_text: string;
      readonly approve: boolean;
      readonly expected_revision: number;
    },
  ): Promise<TaskRecord>;
  startTask(
    taskId: string,
    input?: { readonly prompt?: string; readonly expected_revision?: number },
  ): Promise<{ readonly task: TaskRecord; readonly run: RunRecord; readonly admitted: boolean }>;
  acceptTask(
    taskId: string,
    input?: {
      readonly comment?: string;
      readonly expected_revision?: number;
      readonly plan_version?: number;
      readonly head_sha?: string;
      readonly base_sha?: string;
      readonly policy_digest?: string;
      readonly evidence_digest?: string;
    },
  ): Promise<{ readonly decision_id: string; readonly task: TaskRecord; readonly run: RunRecord }>;
  rejectTask(
    taskId: string,
    input: { readonly reason: string; readonly expected_revision?: number },
  ): Promise<{ readonly decision_id: string; readonly task: TaskRecord; readonly run: RunRecord }>;
  createTaskPr(
    taskId: string,
    input: {
      readonly title?: string;
      readonly base_branch?: string;
      readonly draft?: boolean;
      readonly expected_revision?: number;
    },
  ): Promise<CreatePrOutcome>;
  getRunHistory(runId: string, signal?: AbortSignal): Promise<RunHistoryResponse>;
  getRunDiff(runId: string, signal?: AbortSignal): Promise<RunDiffResponse>;
  verifyRun(runId: string, taskId: string): Promise<unknown>;
  stopRun(input: {
    readonly sessionId: string;
    readonly runId: string;
    readonly expectedRevision: number;
  }): Promise<unknown>;
  resumeSession(input: {
    readonly sessionId: string;
    readonly runId: string;
    readonly expectedRevision: number;
  }): Promise<unknown>;
  answerQuestion(input: {
    readonly sessionId: string;
    readonly questionId: string;
    readonly answer: string;
    readonly expectedRevision: number;
  }): Promise<unknown>;
  dispatchChat(input: {
    readonly input: string;
    readonly project_id?: string;
    readonly session_id?: string;
    readonly run_id?: string;
  }): Promise<ChatDispatchResult>;
  listPlugins(signal?: AbortSignal): Promise<readonly InstalledPlugin[]>;
  installPlugin(path: string): Promise<InstalledPlugin>;
}

export function normalizeRunStatusValue(raw: string | null | undefined): RunStatus {
  if (!raw) return "unknown";
  const lower = raw.replace(/[A-Z]/g, (m, i: number) =>
    i > 0 ? `_${m.toLowerCase()}` : m.toLowerCase(),
  );
  switch (lower) {
    case "queued":
    case "running":
    case "waiting_for_approval":
    case "cancel_requested":
    case "stopped":
    case "completed":
    case "failed":
    case "interrupted":
      return lower;
    default:
      return "unknown";
  }
}

export function hasOwnPendingDecision(entry: BoardTaskEntry): boolean {
  if (entry.pending_questions.some((q) => q.status === "pending")) return true;
  if (entry.task.stage === "acceptance") return true;
  const runStatus = normalizeRunStatusValue(entry.latest_run?.status);
  return runStatus === "waiting_for_approval";
}

export function formatMicroUsd(microUsd: number, locale: Locale): string {
  const usd = microUsd / 1_000_000;
  return new Intl.NumberFormat(locale === "ru" ? "ru-RU" : "en-US", {
    style: "currency",
    currency: "USD",
    minimumFractionDigits: microUsd > 0 && microUsd < 10_000 ? 4 : 2,
    maximumFractionDigits: 4,
  }).format(usd);
}

export function taskStageLabel(stage: TaskStage, t: RunlarkStrings): string {
  switch (stage) {
    case "inbox":
      return t.stageInbox;
    case "ready":
      return t.stageReady;
    case "in_progress":
      return t.stageInProgress;
    case "acceptance":
      return t.stageAcceptance;
    case "done":
      return t.stageDone;
  }
}

export function workflowStageLabel(stage: WorkflowStage, t: RunlarkStrings): string {
  switch (stage) {
    case "clarification":
      return t.wfClarification;
    case "plan":
      return t.wfPlan;
    case "implementation":
      return t.wfImplementation;
    case "verification":
      return t.wfVerification;
    case "human_acceptance":
      return t.wfHumanAcceptance;
  }
}

export function localizedRunStatusLabel(status: RunStatus, t: RunlarkStrings): string {
  switch (status) {
    case "queued":
      return t.statusQueued;
    case "running":
      return t.statusRunning;
    case "waiting_for_approval":
      return t.statusWaiting;
    case "cancel_requested":
      return t.statusCancelling;
    case "stopped":
      return t.statusStopped;
    case "completed":
      return t.statusCompleted;
    case "failed":
      return t.statusFailed;
    case "interrupted":
      return t.statusInterrupted;
    case "unknown":
      return t.statusUnknown;
  }
}
