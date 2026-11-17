import { wrap } from "@reatom/core";
import { reatomComponent, useWrap } from "@reatom/react";
import { useEffect, useId, useState, type FormEvent, type ReactNode } from "react";
import { getStrings } from "@/common/i18n";
import { ActionButton } from "@/common/ui/ActionButton";
import { WorkbenchShell, type BreadcrumbSegment } from "@/common/ui/WorkbenchShell";
import { BoardContentView } from "./BoardWorkspaceView";
import type { TaskFlowModel } from "./model";
import { TaskDetailView } from "./TaskDetailView";

const ProjectsPanel = reatomComponent<{ readonly model: TaskFlowModel }>(({ model }): ReactNode => {
  const strings = getStrings(model.locale());
  const projects = model.projects();
  const pending = model.actionPending();
  const [name, setName] = useState("");
  const [repoRoot, setRepoRoot] = useState("");
  const [branch, setBranch] = useState("main");
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

  return (
    <div className="rl-field" style={{ gap: "0.875rem" }}>
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
          <ul className="rl-event-list">
            {projects.map((p) => (
              <li key={p.project_id} className="rl-event-row">
                <span className="rl-event-type">{p.name}</span>
                <span className="rl-meta-value">{p.default_branch}</span>
                <p className="rl-event-detail rl-mono">{p.repo_root}</p>
              </li>
            ))}
          </ul>
        )}
      </article>
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
  readonly autoStartSync?: boolean;
  readonly pollIntervalMs?: number;
  readonly embedded?: boolean;
  readonly headerActions?: ReactNode;
}

export const TaskFlowWorkspaceView = reatomComponent<TaskFlowWorkspaceViewProps>(
  ({ model, autoStartSync = true, pollIntervalMs, embedded = false, headerActions }): ReactNode => {
    const startSync = useWrap(model.startBoardSync, "TaskFlowWorkspaceView.startBoardSync");

    useEffect(() => {
      if (!autoStartSync) return undefined;
      return startSync(pollIntervalMs);
    }, [startSync, autoStartSync, pollIntervalMs]);

    const locale = model.locale();
    const strings = getStrings(locale);
    const section = model.section();
    const projects = model.projects();
    const selectedProjectId = model.selectedProjectId();
    const selectedTaskId = model.selectedTaskId();
    const selectedDetail = model.selectedTaskDetail();
    const decisionCount = model.decisionEntries().length;

    const sectionLabel =
      section === "decisions"
        ? strings.breadcrumbDecisions
        : section === "projects"
          ? strings.breadcrumbProjects
          : section === "plugins"
            ? strings.breadcrumbPlugins
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
        onSelectSection={wrap((next) => model.navigateTo(next))}
        projects={projects}
        selectedProjectId={selectedProjectId}
        onSelectProject={wrap((pid) => void model.selectProject(pid))}
        decisionCount={decisionCount}
        breadcrumbs={breadcrumbs}
        {...(headerActions !== undefined ? { headerActions } : {})}
        embedded={embedded}
      >
        {selectedTaskId ? (
          <TaskDetailView model={model} />
        ) : section === "projects" ? (
          <ProjectsPanel model={model} />
        ) : section === "plugins" ? (
          <PluginsPanel model={model} />
        ) : (
          <BoardContentView model={model} />
        )}
      </WorkbenchShell>
    );
  },
  "TaskFlowWorkspaceView",
);
