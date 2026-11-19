import { RU } from "./i18n-ru";

export type Locale = "en" | "ru";

export interface RunlarkStrings {
  readonly appTitle: string;
  readonly navGroupWorkspace: string;
  readonly navGroupManage: string;
  readonly navBoard: string;
  readonly navDecisions: string;
  readonly navProjects: string;
  readonly navPlugins: string;
  readonly navTeam: string;
  readonly navMenuOpen: string;
  readonly navMenuClose: string;
  readonly navDrawerLabel: string;
  readonly languageLabel: string;
  readonly languageEn: string;
  readonly languageRu: string;
  readonly projectSelectorLabel: string;
  readonly noProjects: string;
  readonly createProjectTitle: string;
  readonly createProjectBtn: string;
  readonly projectNameLabel: string;
  readonly projectRepoLabel: string;
  readonly projectBranchLabel: string;
  readonly boardHeading: string;
  readonly boardSubtitle: string;
  readonly breadcrumbHome: string;
  readonly breadcrumbBoard: string;
  readonly breadcrumbDecisions: string;
  readonly breadcrumbProjects: string;
  readonly breadcrumbPlugins: string;
  readonly breadcrumbTeam: string;
  readonly slotsSummary: (used: number, max: number, queued: number) => string;
  readonly attentionTitle: (count: number) => string;
  readonly attentionAction: string;
  readonly searchLabel: string;
  readonly searchPlaceholder: string;
  readonly kindFilterLabel: string;
  readonly kindAll: string;
  readonly kindStandard: string;
  readonly kindQuickFix: string;
  readonly decisionsOnly: string;
  readonly newTaskBtn: string;
  readonly newQuickFixBtn: string;
  readonly refreshBtn: string;
  readonly stageInbox: string;
  readonly stageReady: string;
  readonly stageInProgress: string;
  readonly stageAcceptance: string;
  readonly stageDone: string;
  readonly wfClarification: string;
  readonly wfPlan: string;
  readonly wfImplementation: string;
  readonly wfVerification: string;
  readonly wfHumanAcceptance: string;
  readonly statusQueued: string;
  readonly statusRunning: string;
  readonly statusWaiting: string;
  readonly statusCancelling: string;
  readonly statusStopped: string;
  readonly statusCompleted: string;
  readonly statusFailed: string;
  readonly statusInterrupted: string;
  readonly statusUnknown: string;
  readonly emptyColumn: string;
  readonly emptyFiltered: string;
  readonly mobilePrevStage: string;
  readonly mobileNextStage: string;
  readonly mobileStageSelect: string;
  readonly cardDragHandle: (title: string) => string;
  readonly cardMoveMenu: (title: string) => string;
  readonly cardCommentCount: (count: number) => string;
  readonly cardLeaseEpoch: (epoch: number) => string;
  readonly moveToLabel: (stage: string) => string;
  readonly moveDisabledHint: string;
  readonly openTaskLabel: string;
  readonly decisionPendingLabel: string;
  readonly knownCostLabel: string;
  readonly heldCostLabel: string;
  readonly unknownCostLabel: string;
  readonly unknownCostAttempts: (count: number) => string;
  readonly repairAttemptsLabel: string;
  readonly createTaskHeading: string;
  readonly createTaskKindLabel: string;
  readonly createTaskTitleLabel: string;
  readonly createTaskTitleHelp: string;
  readonly createTaskTitleRequired: string;
  readonly createTaskDescLabel: string;
  readonly createTaskSubmit: string;
  readonly cancelBtn: string;
  readonly closeBtn: string;
  readonly backToBoardBtn: string;
  readonly taskTabWorkflow: string;
  readonly taskTabRunBudget: string;
  readonly taskTabDiffArtifacts: string;
  readonly taskTabAcceptancePr: string;
  readonly taskTabComments: string;
  readonly taskTabEventsChat: string;
  readonly clarifyLabel: string;
  readonly clarifySubmit: string;
  readonly planLabel: string;
  readonly planSave: string;
  readonly planSaveApprove: string;
  readonly startRunBtn: string;
  readonly startRunDisabledActive: string;
  readonly stopRunBtn: string;
  readonly resumeRunBtn: string;
  readonly verifyRunBtn: string;
  readonly acceptHeading: string;
  readonly acceptCommentLabel: string;
  readonly acceptBtn: string;
  readonly acceptDisabledHint: string;
  readonly rejectBtn: string;
  readonly rejectReasonLabel: string;
  readonly rejectReasonRequired: string;
  readonly createPrHeading: string;
  readonly prTitleLabel: string;
  readonly prBaseBranchLabel: string;
  readonly prDraftLabel: string;
  readonly createPrBtn: string;
  readonly prDisabledHint: string;
  readonly prUnknownNotice: string;
  readonly chatCommandLabel: string;
  readonly chatCommandPlaceholder: string;
  readonly chatDispatchBtn: string;
  readonly diffEmpty: string;
  readonly artifactsEmpty: string;
  readonly attemptsEmpty: string;
  readonly decisionsEmpty: string;
  readonly installPluginPathLabel: string;
  readonly installPluginBtn: string;
  readonly movedAnnouncement: (title: string, stage: string) => string;
  readonly dropTargetHint: (stage: string) => string;
  readonly sampleFooterNotice: string;
  readonly openFullBoard: string;
  readonly backToSession: string;
  readonly loadingTask: string;
  readonly taskIdLabel: string;
  readonly revisionPlanLabel: string;
  readonly kindMetaLabel: string;
  readonly branchPrLabel: string;
  readonly worktreeDigestLabel: string;
  readonly attemptsHeading: (count: number) => string;
  readonly gitDiffHeading: string;
  readonly casArtifactsHeading: (count: number) => string;
  readonly planVersionLabel: string;
  readonly headBaseShaLabel: string;
  readonly decisionsHeading: (count: number) => string;
  readonly stopDisabledHint: string;
  readonly resumeDisabledHint: string;
  readonly verifyDisabledHint: string;
  readonly humanDecisionRequired: string;
  readonly chooseOptionLegend: string;
  readonly customAnswerOrWrite: string;
  readonly customAnswerLabel: string;
  readonly answerValidationRequired: string;
  readonly liveEventsHeading: string;
  readonly eventsNone: string;
  readonly eventsLoading: string;
  readonly eventsEmpty: string;
}

const EN: RunlarkStrings = {
  appTitle: "Runlark",
  navGroupWorkspace: "WORKBENCH",
  navGroupManage: "ADMINISTRATION",
  navBoard: "Board",
  navDecisions: "Decisions",
  navProjects: "Projects",
  navPlugins: "Plugins",
  navTeam: "Team",
  navMenuOpen: "Open navigation menu",
  navMenuClose: "Close navigation menu",
  navDrawerLabel: "Runlark navigation",
  languageLabel: "Language",
  languageEn: "EN",
  languageRu: "RU",
  projectSelectorLabel: "Project",
  noProjects: "No projects registered yet",
  createProjectTitle: "Register local repository",
  createProjectBtn: "Create project",
  projectNameLabel: "Project name",
  projectRepoLabel: "Repository path",
  projectBranchLabel: "Default branch",
  boardHeading: "Kanban Workbench",
  boardSubtitle: "Solo durable workflow · 2 execution slots · bound human acceptance",
  breadcrumbHome: "Runlark",
  breadcrumbBoard: "Board",
  breadcrumbDecisions: "Decisions",
  breadcrumbProjects: "Projects",
  breadcrumbPlugins: "Plugins",
  breadcrumbTeam: "Team",
  slotsSummary: (used, max, queued) =>
    `${String(used)}/${String(max)} slots · ${String(queued)} queued`,
  attentionTitle: (count) =>
    `${String(count)} item${count === 1 ? "" : "s"} awaiting human review or answer`,
  attentionAction: "Filter pending decisions",
  searchLabel: "Search tasks",
  searchPlaceholder: "Filter by title, ID, or stage",
  kindFilterLabel: "Kind",
  kindAll: "All kinds",
  kindStandard: "Standard",
  kindQuickFix: "Quick Fix",
  decisionsOnly: "My decisions",
  newTaskBtn: "New task",
  newQuickFixBtn: "Quick fix",
  refreshBtn: "Refresh",
  stageInbox: "Inbox",
  stageReady: "Ready",
  stageInProgress: "In Progress",
  stageAcceptance: "Acceptance",
  stageDone: "Done",
  wfClarification: "Clarification",
  wfPlan: "Plan",
  wfImplementation: "Implementation",
  wfVerification: "Verification",
  wfHumanAcceptance: "Human acceptance",
  statusQueued: "Queued",
  statusRunning: "Running",
  statusWaiting: "Waiting for approval",
  statusCancelling: "Cancel requested",
  statusStopped: "Stopped",
  statusCompleted: "Completed",
  statusFailed: "Failed",
  statusInterrupted: "Interrupted · unknown",
  statusUnknown: "Unknown",
  emptyColumn: "No tasks in this column yet.",
  emptyFiltered: "No tasks match the active search or filter.",
  mobilePrevStage: "Previous stage",
  mobileNextStage: "Next stage",
  mobileStageSelect: "Active board stage",
  cardDragHandle: (title) => `Drag ${title} to another stage`,
  cardMoveMenu: (title) => `Move or actions for ${title}`,
  cardCommentCount: (count) => `${String(count)} cmt`,
  cardLeaseEpoch: (epoch) => `e${String(epoch)}`,
  moveToLabel: (stage) => `Move to ${stage}`,
  moveDisabledHint: "Current stage",
  openTaskLabel: "Open task",
  decisionPendingLabel: "Needs human decision",
  knownCostLabel: "Known spend",
  heldCostLabel: "Held",
  unknownCostLabel: "Unknown cost",
  unknownCostAttempts: (count) => `${String(count)} unknown cost`,
  repairAttemptsLabel: "Repair attempts",
  createTaskHeading: "Create task",
  createTaskKindLabel: "Workflow kind",
  createTaskTitleLabel: "Task title",
  createTaskTitleHelp: "Describe the concrete deliverable or bug fix.",
  createTaskTitleRequired: "Enter a task title before creating.",
  createTaskDescLabel: "Description",
  createTaskSubmit: "Create task",
  cancelBtn: "Cancel",
  closeBtn: "Close",
  backToBoardBtn: "Back to board",
  taskTabWorkflow: "Workflow & Plan",
  taskTabRunBudget: "Run & Budget",
  taskTabDiffArtifacts: "Diff & Artifacts",
  taskTabAcceptancePr: "Acceptance & PR",
  taskTabComments: "Comments",
  taskTabEventsChat: "Events & Chat",
  clarifyLabel: "Clarification notes",
  clarifySubmit: "Record clarification",
  planLabel: "Execution plan",
  planSave: "Save plan",
  planSaveApprove: "Save & approve plan",
  startRunBtn: "Start run",
  startRunDisabledActive: "Run already active or queued",
  stopRunBtn: "Request stop",
  resumeRunBtn: "Resume run",
  verifyRunBtn: "Run WASM verification",
  acceptHeading: "Bound human acceptance",
  acceptCommentLabel: "Acceptance note (optional)",
  acceptBtn: "Accept deliverable",
  acceptDisabledHint: "Requires Verification evidence in Acceptance stage",
  rejectBtn: "Request changes",
  rejectReasonLabel: "Required changes reason",
  rejectReasonRequired: "Enter a reason describing requested changes.",
  createPrHeading: "Pull request",
  prTitleLabel: "PR title (optional)",
  prBaseBranchLabel: "Base branch",
  prDraftLabel: "Open as draft PR",
  createPrBtn: "Create PR",
  prDisabledHint: "Requires bound human acceptance first",
  prUnknownNotice: "External PR outcome unknown — check git remote before retrying.",
  chatCommandLabel: "Slash command or prompt",
  chatCommandPlaceholder: "/fix <title>, /plan <id> <text>, /accept <id>, /status",
  chatDispatchBtn: "Run command",
  diffEmpty: "No git diff recorded for this run yet.",
  artifactsEmpty: "No CAS artifacts linked yet.",
  attemptsEmpty: "No execution attempts recorded yet.",
  decisionsEmpty: "No human decisions recorded yet.",
  installPluginPathLabel: "Workflow package directory",
  installPluginBtn: "Install workflow package",
  movedAnnouncement: (title, stage) => `Moved ${title} to ${stage}.`,
  dropTargetHint: (stage) => `Drop to move to ${stage}`,
  sampleFooterNotice: "Runlark local SQLite + CAS authority · live daemon state",
  openFullBoard: "Open Kanban Board",
  backToSession: "Session Card",
  loadingTask: "Loading task...",
  taskIdLabel: "Task ID",
  revisionPlanLabel: "Revision / Plan",
  kindMetaLabel: "Kind",
  branchPrLabel: "Branch / PR",
  worktreeDigestLabel: "Worktree & Workflow Digest",
  attemptsHeading: (count) => `Attempts (${String(count)})`,
  gitDiffHeading: "Git Diff",
  casArtifactsHeading: (count) => `CAS Artifacts (${String(count)})`,
  planVersionLabel: "Plan Version",
  headBaseShaLabel: "Head / Base SHA",
  decisionsHeading: (count) => `Decisions (${String(count)})`,
  stopDisabledHint: "Stop is available only while running or queued",
  resumeDisabledHint: "Completed or stopped runs cannot be resumed",
  verifyDisabledHint: "Verification starts after admission from queue",
  humanDecisionRequired: "Human decision required",
  chooseOptionLegend: "Choose an option",
  customAnswerOrWrite: "Or write a custom answer",
  customAnswerLabel: "Answer",
  answerValidationRequired: "Select an option or enter a response before submitting.",
  liveEventsHeading: "Live event stream",
  eventsNone: "No events",
  eventsLoading: "Loading run event history...",
  eventsEmpty: "No run events persisted in SQLite yet.",
};

export function getStrings(locale: Locale): RunlarkStrings {
  return locale === "ru" ? RU : EN;
}
