import { useId, useState, type FormEvent, type ReactNode } from "react";
import type { TeamStrings } from "../team-i18n";
import type { TeamSessionSummary, TeamWorkspaceRecord } from "../team-types";
import { ActionButton } from "./ActionButton";

export interface TeamConnectionCardProps {
  readonly sessions: readonly TeamSessionSummary[];
  readonly workspaces: readonly TeamWorkspaceRecord[];
  readonly selectedWorkspaceId: string | null;
  readonly busy: boolean;
  readonly strings: TeamStrings;
  readonly onSelectWorkspace: (workspaceId: string) => void;
  readonly onLogin: (input: {
    readonly server_url: string;
    readonly bearer_token: string;
    readonly workspace_id?: string;
  }) => void;
  readonly onAcceptInvitation: (input: {
    readonly server_url: string;
    readonly invite_token: string;
    readonly display_name: string;
    readonly user_id?: string;
  }) => void;
  readonly onCreateWorkspace: (input: { readonly name: string; readonly slug: string }) => void;
}

export function TeamConnectionCard({
  sessions,
  workspaces,
  selectedWorkspaceId,
  busy,
  strings,
  onSelectWorkspace,
  onLogin,
  onAcceptInvitation,
  onCreateWorkspace,
}: TeamConnectionCardProps): ReactNode {
  const [serverUrl, setServerUrl] = useState("http://127.0.0.1:9880");
  const [bearerToken, setBearerToken] = useState("");
  const [loginWsId, setLoginWsId] = useState("");
  const [loginError, setLoginError] = useState<string | null>(null);

  const [invServerUrl, setInvServerUrl] = useState("http://127.0.0.1:9880");
  const [inviteToken, setInviteToken] = useState("");
  const [displayName, setDisplayName] = useState("");
  const [userId, setUserId] = useState("");
  const [acceptError, setAcceptError] = useState<string | null>(null);

  const [wsName, setWsName] = useState("");
  const [wsSlug, setWsSlug] = useState("");
  const [wsError, setWsError] = useState<string | null>(null);

  const serverUrlId = useId();
  const bearerId = useId();
  const loginWsFieldId = useId();
  const loginErrId = useId();
  const invSrvId = useId();
  const invTokId = useId();
  const dispNameId = useId();
  const uidId = useId();
  const acceptErrId = useId();
  const activeWsId = useId();
  const wsNameId = useId();
  const wsSlugId = useId();
  const wsErrId = useId();

  const handleLogin = (e: FormEvent<HTMLFormElement>): void => {
    e.preventDefault();
    if (!serverUrl.trim() || !bearerToken.trim()) {
      setLoginError(strings.fieldRequiredError);
      return;
    }
    setLoginError(null);
    onLogin({
      server_url: serverUrl.trim(),
      bearer_token: bearerToken.trim(),
      ...(loginWsId.trim() ? { workspace_id: loginWsId.trim() } : {}),
    });
    setBearerToken("");
  };

  const handleAccept = (e: FormEvent<HTMLFormElement>): void => {
    e.preventDefault();
    if (!invServerUrl.trim() || !inviteToken.trim() || !displayName.trim()) {
      setAcceptError(strings.fieldRequiredError);
      return;
    }
    setAcceptError(null);
    onAcceptInvitation({
      server_url: invServerUrl.trim(),
      invite_token: inviteToken.trim(),
      display_name: displayName.trim(),
      ...(userId.trim() ? { user_id: userId.trim() } : {}),
    });
    setInviteToken("");
  };

  const handleCreateWs = (e: FormEvent<HTMLFormElement>): void => {
    e.preventDefault();
    if (!wsName.trim() || !wsSlug.trim()) {
      setWsError(strings.fieldRequiredError);
      return;
    }
    setWsError(null);
    onCreateWorkspace({ name: wsName.trim(), slug: wsSlug.trim() });
    setWsName("");
    setWsSlug("");
  };

  return (
    <article className="rl-card" data-testid="rl-team-connection-card">
      <h2 className="rl-section-heading">{strings.serverConnectionTitle}</h2>

      <div>
        <h3 className="rl-meta-label">{strings.connectedSessionsHeading}</h3>
        {sessions.length === 0 ? (
          <p className="rl-status-line">{strings.noConnectedSessions}</p>
        ) : (
          <ul className="rl-team-list" data-testid="rl-team-sessions-list">
            {sessions.map((s) => (
              <li key={s.server_url} className="rl-team-row">
                <div className="rl-team-row-info">
                  <span className="rl-comment-author">
                    {s.display_name} <span className="rl-meta-value">({s.user_id})</span>
                  </span>
                  <span className="rl-meta-value">{s.server_url}</span>
                </div>
                {s.default_workspace_id ? (
                  <span className="rl-badge rl-badge-neutral rl-mono">
                    {s.default_workspace_id}
                  </span>
                ) : null}
              </li>
            ))}
          </ul>
        )}
      </div>

      {workspaces.length > 0 ? (
        <div className="rl-field">
          <label htmlFor={activeWsId} className="rl-label">
            {strings.activeWorkspaceLabel}
          </label>
          <select
            id={activeWsId}
            className="rl-toolbar-select"
            value={selectedWorkspaceId ?? workspaces[0]?.workspace.id ?? ""}
            onChange={(e) => onSelectWorkspace(e.currentTarget.value)}
            data-testid="rl-team-workspace-select"
          >
            {workspaces.map((w) => (
              <option key={w.workspace.id} value={w.workspace.id}>
                {w.workspace.name} ({w.workspace.id} · #{String(w.last_cursor_seq)})
              </option>
            ))}
          </select>
        </div>
      ) : null}

      <div className="rl-team-split">
        <form
          onSubmit={handleLogin}
          className="rl-field"
          noValidate
          data-testid="rl-team-login-form"
        >
          <h3 className="rl-card-title">{strings.connectServerBtn}</h3>
          <div className="rl-field">
            <label htmlFor={serverUrlId} className="rl-label">
              {strings.serverUrlLabel}
            </label>
            <input
              id={serverUrlId}
              type="url"
              className="rl-input"
              value={serverUrl}
              disabled={busy}
              aria-invalid={loginError && !serverUrl.trim() ? "true" : "false"}
              aria-describedby={loginError ? loginErrId : undefined}
              onChange={(e) => {
                setServerUrl(e.currentTarget.value);
                if (loginError) setLoginError(null);
              }}
              data-testid="rl-login-server-url"
            />
          </div>
          <div className="rl-field">
            <label htmlFor={bearerId} className="rl-label">
              {strings.bearerTokenLabel}
            </label>
            <input
              id={bearerId}
              type="password"
              autoComplete="off"
              className="rl-input"
              value={bearerToken}
              disabled={busy}
              aria-invalid={loginError && !bearerToken.trim() ? "true" : "false"}
              aria-describedby={loginError ? loginErrId : undefined}
              onChange={(e) => {
                setBearerToken(e.currentTarget.value);
                if (loginError) setLoginError(null);
              }}
              data-testid="rl-login-bearer-token"
            />
          </div>
          <div className="rl-field">
            <label htmlFor={loginWsFieldId} className="rl-label">
              {strings.workspaceIdOptionalLabel}
            </label>
            <input
              id={loginWsFieldId}
              type="text"
              className="rl-input"
              value={loginWsId}
              disabled={busy}
              onChange={(e) => setLoginWsId(e.currentTarget.value)}
              data-testid="rl-login-workspace-id"
            />
          </div>
          {loginError ? (
            <div id={loginErrId} className="rl-error-box" role="alert">
              {loginError}
            </div>
          ) : null}
          <div className="rl-actions-row">
            <ActionButton
              type="submit"
              variant="primary"
              label={strings.connectServerBtn}
              disabled={busy}
              testId="rl-login-submit-btn"
            />
          </div>
        </form>

        <form
          onSubmit={handleAccept}
          className="rl-field"
          noValidate
          data-testid="rl-accept-invite-form"
        >
          <h3 className="rl-card-title">{strings.acceptInviteHeading}</h3>
          <div className="rl-field">
            <label htmlFor={invSrvId} className="rl-label">
              {strings.serverUrlLabel}
            </label>
            <input
              id={invSrvId}
              type="url"
              className="rl-input"
              value={invServerUrl}
              disabled={busy}
              aria-invalid={acceptError && !invServerUrl.trim() ? "true" : "false"}
              aria-describedby={acceptError ? acceptErrId : undefined}
              onChange={(e) => {
                setInvServerUrl(e.currentTarget.value);
                if (acceptError) setAcceptError(null);
              }}
              data-testid="rl-accept-invite-server"
            />
          </div>
          <div className="rl-field">
            <label htmlFor={invTokId} className="rl-label">
              {strings.inviteTokenLabel}
            </label>
            <input
              id={invTokId}
              type="text"
              className="rl-input rl-mono"
              value={inviteToken}
              disabled={busy}
              aria-invalid={acceptError && !inviteToken.trim() ? "true" : "false"}
              aria-describedby={acceptError ? acceptErrId : undefined}
              onChange={(e) => {
                setInviteToken(e.currentTarget.value);
                if (acceptError) setAcceptError(null);
              }}
              data-testid="rl-accept-invite-token"
            />
          </div>
          <div className="rl-field">
            <label htmlFor={dispNameId} className="rl-label">
              {strings.displayNameLabel}
            </label>
            <input
              id={dispNameId}
              type="text"
              className="rl-input"
              value={displayName}
              disabled={busy}
              aria-invalid={acceptError && !displayName.trim() ? "true" : "false"}
              aria-describedby={acceptError ? acceptErrId : undefined}
              onChange={(e) => {
                setDisplayName(e.currentTarget.value);
                if (acceptError) setAcceptError(null);
              }}
              data-testid="rl-accept-invite-name"
            />
          </div>
          <div className="rl-field">
            <label htmlFor={uidId} className="rl-label">
              {strings.userIdOptionalLabel}
            </label>
            <input
              id={uidId}
              type="text"
              className="rl-input rl-mono"
              value={userId}
              disabled={busy}
              onChange={(e) => setUserId(e.currentTarget.value)}
            />
          </div>
          {acceptError ? (
            <div id={acceptErrId} className="rl-error-box" role="alert">
              {acceptError}
            </div>
          ) : null}
          <div className="rl-actions-row">
            <ActionButton
              type="submit"
              variant="secondary"
              label={strings.acceptInviteBtn}
              disabled={busy}
              testId="rl-accept-invite-submit"
            />
          </div>
        </form>
      </div>

      {sessions.length > 0 ? (
        <form
          onSubmit={handleCreateWs}
          className="rl-team-split"
          noValidate
          data-testid="rl-create-workspace-form"
        >
          <div className="rl-field">
            <label htmlFor={wsNameId} className="rl-label">
              {strings.workspaceNameLabel}
            </label>
            <input
              id={wsNameId}
              type="text"
              className="rl-input"
              value={wsName}
              disabled={busy}
              aria-invalid={wsError && !wsName.trim() ? "true" : "false"}
              aria-describedby={wsError ? wsErrId : undefined}
              onChange={(e) => {
                setWsName(e.currentTarget.value);
                if (wsError) setWsError(null);
              }}
              data-testid="rl-create-ws-name"
            />
          </div>
          <div className="rl-field">
            <label htmlFor={wsSlugId} className="rl-label">
              {strings.workspaceSlugLabel}
            </label>
            <input
              id={wsSlugId}
              type="text"
              className="rl-input rl-mono"
              value={wsSlug}
              disabled={busy}
              aria-invalid={wsError && !wsSlug.trim() ? "true" : "false"}
              aria-describedby={wsError ? wsErrId : undefined}
              onChange={(e) => {
                setWsSlug(e.currentTarget.value);
                if (wsError) setWsError(null);
              }}
              data-testid="rl-create-ws-slug"
            />
          </div>
          {wsError ? (
            <div id={wsErrId} className="rl-error-box" role="alert">
              {wsError}
            </div>
          ) : null}
          <div className="rl-actions-row">
            <ActionButton
              type="submit"
              variant="secondary"
              label={strings.createWorkspaceBtn}
              disabled={busy}
              testId="rl-create-ws-submit"
            />
          </div>
        </form>
      ) : null}
    </article>
  );
}
