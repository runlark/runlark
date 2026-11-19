import { wrap } from "@reatom/core";
import { reatomComponent, useWrap } from "@reatom/react";
import { useEffect, useId, useState, type FormEvent, type ReactNode } from "react";
import { normalizeRunStatusValue, type ProjectSummary } from "@/common/board-types";
import { getStrings } from "@/common/i18n";
import { getTeamStrings } from "@/common/team-i18n";
import { isTeamManagedMode } from "@/common/team-types";
import { ActionButton } from "@/common/ui/ActionButton";
import { PromoteTeamDialog } from "@/common/ui/PromoteTeamDialog";
import { StatusBadge } from "@/common/ui/StatusBadge";
import { TeamSyncStatusBar } from "@/common/ui/TeamSyncStatusBar";
import { WorkbenchShell, type BreadcrumbSegment } from "@/common/ui/WorkbenchShell";
import { TeamWorkspaceView, type TeamCollabModel } from "@/modules/team-collab";
import { BoardContentView } from "./BoardWorkspaceView";
import type { TaskFlowModel } from "./model";
import { TaskDetailView } from "./TaskDetailView";

const ProjectsPanel = reatomComponent<{
  readonly model: TaskFlowModel;
  readonly teamModel: TeamCollabModel;
}>(({ model, teamModel }): ReactNode => {
  const locale = model.locale();
  const strings = getStrings(locale);
  const teamStrings = getTeamStrings(locale);
  const projects = model.projects();
  const board = model.board();
  const pending = model.actionPending();
  const teamPending = teamModel.actionPending();
  const teamStatus = teamModel.teamStatusResource.data();
  const [name, setName] = useState("");
  const [repoRoot, setRepoRoot] = useState("");
  const [branch, setBranch] = useState("main");
  const [promoteTarget, setPromoteTarget] = useState<ProjectSummary | null>(null);
  const nameId = useId();
  const repoId = useId();
  const branchId = useId();

  const submitCreate = useWrap(
    (input: {
      readonly name: string;
      readonly repo_root: string;
      readonly default_branch: string;
    }) => void model.createProject(input),
    "ProjectsPanel.createProject",
  );

  const submitPromote = useWrap(
    async (input: { readonly projectId: string; readonly workspaceId: string }) => {
      const ok = await teamModel.promoteProject(input);
      if (ok) {
        setPromoteTarget(null);
      }
    },
    "ProjectsPanel.promoteProject",
  );

  const handleSubmit = (e: FormEvent<HTMLFormElement>): void => {
    e.preventDefault();
    if (!name.trim() || !repoRoot.trim()) return;
    submitCreate({
      name: name.trim(),
      repo_root: repoRoot.trim(),
      default_branch: branch.trim() || "main",
    });
    setName("");
    setRepoRoot("");
  };

  const blockingRunIdsFor = (projectId: string): readonly string[] => {
    if (!board || board.project?.project_id !== projectId) return [];
    const ids = new Set<string>();
    const isTerminal = (status: string | undefined): boolean => {
      const s = normalizeRunStatusValue(status);
      return s === "completed" || s === "stopped" || s === "failed";
    };
    for (const r of board.active_runs) {
      if (!isTerminal(r.status)) ids.add(r.id);
    }
    for (const r of board.queued_runs) {
      if (!isTerminal(r.status)) ids.add(r.id);
    }
    for (const entry of board.tasks) {
      if (entry.latest_run && !isTerminal(entry.latest_run.status)) {
        ids.add(entry.latest_run.id);
      }
    }
    return [...ids];
  };

  const blockingIds = promoteTarget ? blockingRunIdsFor(promoteTarget.project_id) : [];

  return (
    <div className="rl-field" style={{ gap: "0.875rem" }} data-testid="rl-projects-panel">
      <h1 className="rl-page-h1">{strings.navProjects}</h1>
      <article className="rl-card">
        <h2 className="rl-section-heading">{strings.createProjectTitle}</h2>
        <form onSubmit={handleSubmit} className="rl-field" style={{ gap: "0.625rem" }}>
          <div className="rl-field">
            <label htmlFor={nameId} className="rl-label">
              {strings.projectNameLabel}
            </label>
            <input
              id={nameId}
              type="text"
              className="rl-input"
              value={name}
              onChange={(e) => setName(e.currentTarget.value)}
            />
          </div>
          <div className="rl-field">
            <label htmlFor={repoId} className="rl-label">
              {strings.projectRepoLabel}
            </label>
            <input
              id={repoId}
              type="text"
              className="rl-input"
              value={repoRoot}
              onChange={(e) => setRepoRoot(e.currentTarget.value)}
            />
          </div>
          <div className="rl-field">
            <label htmlFor={branchId} className="rl-label">
              {strings.projectBranchLabel}
            </label>
            <input
              id={branchId}
              type="text"
              className="rl-input"
              value={branch}
              onChange={(e) => setBranch(e.currentTarget.value)}
            />
          </div>
          <div className="rl-actions-row">
            <ActionButton
              type="submit"
              variant="primary"
              label={strings.createProjectBtn}
              disabled={pending !== null || !name.trim() || !repoRoot.trim()}
            />
          </div>
        </form>
      </article>

      <article className="rl-card">
        {projects.length === 0 ? (
          <p className="rl-status-line">{strings.noProjects}</p>
        ) : (
          <ul className="rl-team-list" data-testid="rl-projects-list">
            {projects.map((p) => {
              const ownership = p.ownership ?? { mode: "solo_local" as const };
              const isTeam = isTeamManagedMode(ownership);
              return (
                <li
                  key={p.project_id}
                  className="rl-team-row"
                  data-testid={`rl-project-row-${p.project_id}`}
                >
                  <div className="rl-team-row-info">
                    <span className="rl-comment-author">
                      {p.name} <span className="rl-meta-value">({p.default_branch})</span>
                    </span>
                    <span className="rl-meta-value">{p.repo_root}</span>
                  </div>
                  <div className="rl-actions-row">
                    <StatusBadge
                      status={isTeam ? "completed" : "queued"}
                      customTone={isTeam ? "success" : "neutral"}
                      customLabel={
                        isTeam
                          ? teamStrings.ownershipTeam(
                              ownership.workspace_id,
                              ownership.ownership_epoch,
                            )
                          : teamStrings.ownershipSolo
                      }
                    />
                    {!isTeam ? (
                      <ActionButton
                        variant="secondary"
                        label={teamStrings.promoteToTeamBtn}
                        disabled={pending !== null || teamPending !== null}
                        onClick={wrap(() => {
                          teamModel.actionError.set(null);
                          setPromoteTarget(p);
                        })}
                        testId={`rl-promote-open-${p.project_id}`}
                      />
                    ) : null}
                  </div>
                </li>
              );
            })}
          </ul>
        )}
      </article>

      <PromoteTeamDialog
        open={promoteTarget !== null}
        project={promoteTarget}
        blockingRunIds={blockingIds}
        workspaces={teamStatus.workspaces}
        submitting={teamPending === "promoteProject"}
        submitError={teamModel.actionError()}
        strings={teamStrings}
        onClose={() => setPromoteTarget(null)}
        onPromote={(input) => void submitPromote(input)}
      />
    </div>
  );
}, "ProjectsPanel");

const PluginsPanel = reatomComponent<{ readonly model: TaskFlowModel }>(({ model }): ReactNode => {
  const strings = getStrings(model.locale());
  const plugins = model.plugins();
  const pending = model.actionPending();
  const [pkgPath, setPkgPath] = useState("workflows/solo-default");
  const pathId = useId();

  const submitInstall = useWrap(
    (path: string) => void model.installPlugin(path),
    "PluginsPanel.installPlugin",
  );

  const handleInstall = (e: FormEvent<HTMLFormElement>): void => {
    e.preventDefault();
    if (!pkgPath.trim()) return;
    submitInstall(pkgPath.trim());
  };

  return (
    <div className="rl-field" style={{ gap: "0.875rem" }}>
      <h1 className="rl-page-h1">{strings.navPlugins}</h1>
      <article className="rl-card">
        <form onSubmit={handleInstall} className="rl-field">
          <label htmlFor={pathId} className="rl-label">
            {strings.installPluginPathLabel}
          </label>
          <div className="rl-actions-row">
            <input
              id={pathId}
              type="text"
              className="rl-input"
              style={{ flex: "1 1 16rem" }}
              value={pkgPath}
              onChange={(e) => setPkgPath(e.currentTarget.value)}
            />
            <ActionButton
              type="submit"
              variant="primary"
              label={strings.installPluginBtn}
              disabled={pending !== null || !pkgPath.trim()}
            />
          </div>
        </form>
      </article>

      <article className="rl-card">
        <ul className="rl-event-list">
          {plugins.map((pl) => (
            <li key={`${pl.name}-${pl.package_digest}`} className="rl-event-row">
              <span className="rl-event-type">
                {pl.name}@{pl.version}
              </span>
              <p className="rl-event-detail rl-mono">{pl.package_digest}</p>
            </li>
          ))}
        </ul>
      </article>
    </div>
  );
}, "PluginsPanel");

export interface TaskFlowWorkspaceViewProps {
  readonly model: TaskFlowModel;
  readonly teamModel: TeamCollabModel;
  readonly autoStartSync?: boolean;
  readonly pollIntervalMs?: number;
  readonly embedded?: boolean;
  readonly headerActions?: ReactNode;
}

export const TaskFlowWorkspaceView = reatomComponent<TaskFlowWorkspaceViewProps>(
  ({
    model,
    teamModel,
    autoStartSync = true,
    pollIntervalMs,
    embedded = false,
    headerActions,
  }): ReactNode => {
    const startSync = useWrap(model.startBoardSync, "TaskFlowWorkspaceView.startBoardSync");

    useEffect(() => {
      if (!autoStartSync) return undefined;
      return startSync(pollIntervalMs);
    }, [startSync, autoStartSync, pollIntervalMs]);

    const locale = model.locale();
    const strings = getStrings(locale);
    const teamStrings = getTeamStrings(locale);
    const section = model.section();
    const projects = model.projects();
    const selectedProjectId = model.selectedProjectId();
    const selectedTaskId = model.selectedTaskId();
    const selectedDetail = model.selectedTaskDetail();
    const decisionCount = model.decisionEntries().length;
    const board = model.board();

    const ownershipMode = board?.ownership_mode ??
      board?.project?.ownership ?? { mode: "solo_local" as const };
    const cursorSeq = board?.last_cursor_seq ?? board?.project?.last_cursor_seq ?? 0;
    const unresolvedDrafts = teamModel.unresolvedDraftCount();
    const conflictCount = teamModel.conflictDraftCount();
    const teamBadgeCount = Math.max(unresolvedDrafts, conflictCount);

    const sectionLabel =
      section === "decisions"
        ? strings.breadcrumbDecisions
        : section === "projects"
          ? strings.breadcrumbProjects
          : section === "plugins"
            ? strings.breadcrumbPlugins
            : section === "team"
              ? strings.breadcrumbTeam
              : strings.breadcrumbBoard;

    const breadcrumbs: BreadcrumbSegment[] = [
      { label: strings.breadcrumbHome, onClick: wrap(() => model.navigateTo("board")) },
      {
        label: sectionLabel,
        ...(selectedTaskId ? { onClick: wrap(() => model.closeTask()) } : {}),
      },
    ];
    if (selectedTaskId) {
      breadcrumbs.push({ label: selectedDetail?.task.title ?? selectedTaskId });
    }

    return (
      <WorkbenchShell
        strings={strings}
        locale={locale}
        onLocaleChange={wrap((next) => model.setLocale(next))}
        section={section}
        onSelectSection={wrap((next) => {
          model.navigateTo(next);
          if (next === "team") {
            teamModel.refreshTeam();
          }
        })}
        projects={projects}
        selectedProjectId={selectedProjectId}
        onSelectProject={wrap((pid) => void model.selectProject(pid))}
        decisionCount={decisionCount}
        teamBadgeCount={teamBadgeCount}
        breadcrumbs={breadcrumbs}
        topBannerSlot={
          <TeamSyncStatusBar
            ownershipMode={ownershipMode}
            cursorSeq={cursorSeq}
            connectionState={teamModel.connectionState()}
            unresolvedDraftsCount={unresolvedDrafts}
            conflictCount={conflictCount}
            conflictNotice={teamModel.conflictNotice()}
            announcement={teamModel.announcement()}
            syncing={teamModel.actionPending() === "syncNow"}
            strings={teamStrings}
            onSyncNow={wrap(() => void teamModel.syncNow())}
            onOpenTeamQueue={wrap(() => {
              model.navigateTo("team");
              teamModel.refreshTeam();
            })}
            onDismissConflict={wrap(() => teamModel.dismissConflict())}
          />
        }
        {...(headerActions !== undefined ? { headerActions } : {})}
        embedded={embedded}
      >
        {selectedTaskId ? (
          <TaskDetailView model={model} teamModel={teamModel} />
        ) : section === "projects" ? (
          <ProjectsPanel model={model} teamModel={teamModel} />
        ) : section === "plugins" ? (
          <PluginsPanel model={model} />
        ) : section === "team" ? (
          <TeamWorkspaceView model={teamModel} locale={locale} />
        ) : (
          <BoardContentView model={model} teamModel={teamModel} />
        )}
      </WorkbenchShell>
    );
  },
  "TaskFlowWorkspaceView",
);
