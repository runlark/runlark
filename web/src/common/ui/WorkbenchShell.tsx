import { useEffect, useId, useRef, useState, type ReactNode } from "react";
import type { ProjectSummary } from "../board-types";
import type { Locale, RunlarkStrings } from "../i18n";
import {
  BoardIconSvg,
  CloseIconSvg,
  DecisionsIconSvg,
  MenuIconSvg,
  PluginsIconSvg,
  ProjectsIconSvg,
  RunlarkLockupSvg,
  RunlarkMarkSvg,
  TeamIconSvg,
} from "./SvgIcons";

export type WorkbenchSection = "board" | "decisions" | "projects" | "plugins" | "team";

export interface BreadcrumbSegment {
  readonly label: string;
  readonly onClick?: () => void;
}

export interface WorkbenchShellProps {
  readonly strings: RunlarkStrings;
  readonly locale: Locale;
  readonly onLocaleChange: (locale: Locale) => void;
  readonly section: WorkbenchSection;
  readonly onSelectSection: (section: WorkbenchSection) => void;
  readonly projects: readonly ProjectSummary[];
  readonly selectedProjectId: string | null;
  readonly onSelectProject: (projectId: string) => void;
  readonly decisionCount: number;
  readonly teamBadgeCount?: number;
  readonly breadcrumbs: readonly BreadcrumbSegment[];
  readonly headerActions?: ReactNode;
  readonly topBannerSlot?: ReactNode;
  readonly embedded?: boolean;
  readonly children: ReactNode;
}

export function WorkbenchShell({
  strings,
  locale,
  onLocaleChange,
  section,
  onSelectSection,
  projects,
  selectedProjectId,
  onSelectProject,
  decisionCount,
  teamBadgeCount,
  breadcrumbs,
  headerActions,
  topBannerSlot,
  embedded = false,
  children,
}: WorkbenchShellProps): ReactNode {
  const [drawerOpen, setDrawerOpen] = useState(false);
  const dialogRef = useRef<HTMLDialogElement | null>(null);
  const menuBtnRef = useRef<HTMLButtonElement | null>(null);
  const drawerId = useId();
  const projectLabelId = useId();

  useEffect(() => {
    const dialog = dialogRef.current;
    if (!dialog) return;
    if (drawerOpen) {
      if (!dialog.open) {
        dialog.showModal();
      }
    } else if (dialog.open) {
      dialog.close();
    }
  }, [drawerOpen]);

  useEffect(() => {
    const dialog = dialogRef.current;
    if (!dialog) return undefined;
    const handleBackdropClick = (event: MouseEvent): void => {
      if (event.target === dialog) {
        setDrawerOpen(false);
      }
    };
    dialog.addEventListener("click", handleBackdropClick);
    return () => {
      dialog.removeEventListener("click", handleBackdropClick);
    };
  }, []);

  const navItems: readonly {
    readonly id: WorkbenchSection;
    readonly label: string;
    readonly icon: ReactNode;
    readonly count?: number;
  }[] = [
    { id: "board", label: strings.navBoard, icon: <BoardIconSvg /> },
    {
      id: "decisions",
      label: strings.navDecisions,
      icon: <DecisionsIconSvg />,
      count: decisionCount,
    },
    {
      id: "projects",
      label: strings.navProjects,
      icon: <ProjectsIconSvg />,
      count: projects.length,
    },
    { id: "plugins", label: strings.navPlugins, icon: <PluginsIconSvg /> },
    {
      id: "team",
      label: strings.navTeam,
      icon: <TeamIconSvg />,
      ...(teamBadgeCount && teamBadgeCount > 0 ? { count: teamBadgeCount } : {}),
    },
  ];

  const renderRailContent = (inDrawer: boolean): ReactNode => (
    <>
      <div className="rl-rail-scroll">
        {projects.length > 0 ? (
          <div className="rl-field">
            <label
              htmlFor={`${projectLabelId}-${inDrawer ? "m" : "d"}`}
              className="rl-rail-group-label"
            >
              {strings.projectSelectorLabel}
            </label>
            <select
              id={`${projectLabelId}-${inDrawer ? "m" : "d"}`}
              className="rl-rail-select"
              value={selectedProjectId ?? projects[0]?.project_id ?? ""}
              onChange={(e) => onSelectProject(e.currentTarget.value)}
            >
              {projects.map((p) => (
                <option key={p.project_id} value={p.project_id}>
                  {p.name}
                </option>
              ))}
            </select>
          </div>
        ) : null}

        <div className="rl-rail-group">
          <p className="rl-rail-group-label">{strings.navGroupWorkspace}</p>
          {navItems.slice(0, 2).map((item) => (
            <button
              key={item.id}
              type="button"
              className="rl-rail-item"
              aria-current={section === item.id ? "page" : undefined}
              data-testid={inDrawer ? `rl-drawer-nav-${item.id}` : `rl-nav-${item.id}`}
              onClick={() => {
                onSelectSection(item.id);
                if (inDrawer) setDrawerOpen(false);
              }}
            >
              <span className="rl-rail-item-inner">
                {item.icon}
                <span>{item.label}</span>
              </span>
              {item.count !== undefined ? (
                <span className="rl-rail-count">{item.count}</span>
              ) : null}
            </button>
          ))}
        </div>

        <div className="rl-rail-group">
          <p className="rl-rail-group-label">{strings.navGroupManage}</p>
          {navItems.slice(2).map((item) => (
            <button
              key={item.id}
              type="button"
              className="rl-rail-item"
              aria-current={section === item.id ? "page" : undefined}
              data-testid={inDrawer ? `rl-drawer-nav-${item.id}` : `rl-nav-${item.id}`}
              onClick={() => {
                onSelectSection(item.id);
                if (inDrawer) setDrawerOpen(false);
              }}
            >
              <span className="rl-rail-item-inner">
                {item.icon}
                <span>{item.label}</span>
              </span>
              {item.count !== undefined ? (
                <span className="rl-rail-count">{item.count}</span>
              ) : null}
            </button>
          ))}
        </div>
      </div>

      <div className="rl-rail-footer">
        <span className="rl-rail-group-label">{strings.languageLabel}</span>
        <fieldset className="rl-lang-switch" aria-label={strings.languageLabel}>
          <button
            type="button"
            className="rl-lang-btn"
            aria-pressed={locale === "en"}
            onClick={() => onLocaleChange("en")}
            data-testid="rl-lang-en"
          >
            {strings.languageEn}
          </button>
          <button
            type="button"
            className="rl-lang-btn"
            aria-pressed={locale === "ru"}
            onClick={() => onLocaleChange("ru")}
            data-testid="rl-lang-ru"
          >
            {strings.languageRu}
          </button>
        </fieldset>
      </div>
    </>
  );

  return (
    <div
      className={embedded ? "rl-shell rl-shell-embedded" : "rl-shell"}
      data-testid="rl-workbench-shell"
    >
      <header className="rl-mobile-header">
        <div className="rl-mobile-brand">
          <RunlarkMarkSvg size={28} />
          <span>{strings.appTitle}</span>
        </div>
        <button
          ref={menuBtnRef}
          type="button"
          className="rl-mobile-menu-btn"
          aria-label={strings.navMenuOpen}
          aria-expanded={drawerOpen}
          aria-controls={drawerId}
          onClick={() => setDrawerOpen(true)}
        >
          <MenuIconSvg />
        </button>
      </header>

      <aside className="rl-rail" aria-label={strings.navDrawerLabel}>
        <RunlarkLockupSvg />
        {renderRailContent(false)}
      </aside>

      <dialog
        ref={dialogRef}
        id={drawerId}
        className="rl-drawer"
        aria-label={strings.navDrawerLabel}
        onCancel={(e) => {
          e.preventDefault();
          setDrawerOpen(false);
        }}
        onClose={() => {
          setDrawerOpen(false);
          menuBtnRef.current?.focus();
        }}
      >
        <div className="rl-drawer-sheet">
          <div className="rl-drawer-head">
            <RunlarkLockupSvg />
            <button
              type="button"
              className="rl-mobile-menu-btn"
              aria-label={strings.navMenuClose}
              onClick={() => setDrawerOpen(false)}
            >
              <CloseIconSvg />
            </button>
          </div>
          {renderRailContent(true)}
        </div>
      </dialog>

      <div className="rl-workspace">
        <div className="rl-breadcrumb-bar">
          <nav aria-label="Breadcrumb">
            <ol className="rl-breadcrumb-list">
              {breadcrumbs.map((crumb, idx) => {
                const isLast = idx === breadcrumbs.length - 1;
                return (
                  <li key={`${crumb.label}-${String(idx)}`}>
                    {idx > 0 ? <span aria-hidden="true"> / </span> : null}
                    {isLast || !crumb.onClick ? (
                      <span
                        className={isLast ? "rl-breadcrumb-current" : undefined}
                        aria-current={isLast ? "page" : undefined}
                      >
                        {crumb.label}
                      </span>
                    ) : (
                      <button type="button" className="rl-breadcrumb-link" onClick={crumb.onClick}>
                        {crumb.label}
                      </button>
                    )}
                  </li>
                );
              })}
            </ol>
          </nav>
          {headerActions ? <div className="rl-actions-row">{headerActions}</div> : null}
        </div>

        {topBannerSlot ?? null}

        <main className="rl-main-content">{children}</main>

        <footer className="rl-footer-strip">{strings.sampleFooterNotice}</footer>
      </div>

      <nav className="rl-mobile-nav" aria-label={strings.navDrawerLabel}>
        {navItems.map((item) => (
          <button
            key={item.id}
            type="button"
            className="rl-mobile-nav-btn"
            aria-current={section === item.id ? "page" : undefined}
            data-testid={`rl-mobile-nav-${item.id}`}
            onClick={() => onSelectSection(item.id)}
          >
            {item.icon}
            <span>{item.label}</span>
          </button>
        ))}
      </nav>
    </div>
  );
}
