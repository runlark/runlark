import { useId, useState, type FormEvent, type ReactNode } from "react";
import type { TeamStrings } from "../team-i18n";
import type {
  CreateInvitationResponse,
  WorkspaceInvitation,
  WorkspaceMember,
  WorkspaceRole,
} from "../team-types";
import { ActionButton } from "./ActionButton";
import { StatusBadge } from "./StatusBadge";

const ALL_ROLES: readonly WorkspaceRole[] = ["owner", "maintainer", "member", "viewer"];
const INVITE_ROLES: readonly WorkspaceRole[] = ["maintainer", "member", "viewer"];

export interface TeamMembersInvitesPanelProps {
  readonly workspaceId: string | null;
  readonly members: readonly WorkspaceMember[];
  readonly invitations: readonly WorkspaceInvitation[];
  readonly invitationsForbidden: boolean;
  readonly lastCreatedInvite: CreateInvitationResponse | null;
  readonly canManageMembers: boolean;
  readonly busy: boolean;
  readonly strings: TeamStrings;
  readonly onUpdateMemberRole: (userId: string, role: WorkspaceRole) => void;
  readonly onCreateInvitation: (input: {
    readonly invitee_hint: string;
    readonly role: WorkspaceRole;
  }) => void;
  readonly onRevokeInvitation: (inviteId: string) => void;
}

export function TeamMembersInvitesPanel({
  workspaceId,
  members,
  invitations,
  invitationsForbidden,
  lastCreatedInvite,
  canManageMembers,
  busy,
  strings,
  onUpdateMemberRole,
  onCreateInvitation,
  onRevokeInvitation,
}: TeamMembersInvitesPanelProps): ReactNode {
  const [inviteeHint, setInviteeHint] = useState("");
  const [inviteRole, setInviteRole] = useState<WorkspaceRole>("member");
  const [inviteError, setInviteError] = useState<string | null>(null);

  const hintInputId = useId();
  const roleSelectId = useId();
  const inviteErrId = useId();
  const manageDeniedHintId = useId();
  const lastOwnerHintId = useId();

  if (!workspaceId) {
    return null;
  }

  const ownerCount = members.filter((m) => m.role === "owner").length;

  const handleInviteSubmit = (e: FormEvent<HTMLFormElement>): void => {
    e.preventDefault();
    if (!inviteeHint.trim()) {
      setInviteError(strings.fieldRequiredError);
      return;
    }
    setInviteError(null);
    onCreateInvitation({ invitee_hint: inviteeHint.trim(), role: inviteRole });
    setInviteeHint("");
  };

  return (
    <article className="rl-card" data-testid="rl-team-members-invites-card">
      <div className="rl-field" style={{ gap: "0.5rem" }}>
        <h2 className="rl-section-heading">{strings.membersHeading(members.length)}</h2>
        {!canManageMembers ? (
          <p id={manageDeniedHintId} className="rl-status-line">
            {strings.roleManageDeniedHint}
          </p>
        ) : null}
        {members.length === 0 ? (
          <p className="rl-status-line">{strings.membersEmpty}</p>
        ) : (
          <ul className="rl-team-list" data-testid="rl-team-members-list">
            {members.map((member) => {
              const isSoleOwner = member.role === "owner" && ownerCount <= 1;
              const disabled = busy || !canManageMembers || isSoleOwner;
              const describedBy = !canManageMembers
                ? manageDeniedHintId
                : isSoleOwner
                  ? lastOwnerHintId
                  : undefined;

              return (
                <li
                  key={member.user_id}
                  className="rl-team-row"
                  data-testid={`rl-member-row-${member.user_id}`}
                >
                  <div className="rl-team-row-info">
                    <span className="rl-comment-author">{member.display_name}</span>
                    <span className="rl-meta-value">{member.user_id}</span>
                  </div>
                  <div className="rl-actions-row">
                    <StatusBadge
                      status="queued"
                      customTone={member.role === "owner" ? "attention" : "neutral"}
                      customLabel={strings.roleLabel(member.role)}
                    />
                    <select
                      className="rl-toolbar-select"
                      aria-label={strings.roleSelectLabel(member.display_name)}
                      aria-describedby={describedBy}
                      value={member.role}
                      disabled={disabled}
                      onChange={(e) =>
                        onUpdateMemberRole(member.user_id, e.currentTarget.value as WorkspaceRole)
                      }
                      data-testid={`rl-member-role-${member.user_id}`}
                    >
                      {ALL_ROLES.map((role) => (
                        <option key={role} value={role}>
                          {strings.roleLabel(role)}
                        </option>
                      ))}
                    </select>
                  </div>
                </li>
              );
            })}
          </ul>
        )}
        {ownerCount === 1 ? (
          <span id={lastOwnerHintId} className="rl-meta-label">
            {strings.lastOwnerHint}
          </span>
        ) : null}
      </div>

      <div className="rl-team-split">
        <div className="rl-field" style={{ gap: "0.5rem" }}>
          <h3 className="rl-card-title">{strings.createInviteBtn}</h3>
          {invitationsForbidden ? (
            <p className="rl-status-line">{strings.invitationsForbiddenHint}</p>
          ) : (
            <form
              onSubmit={handleInviteSubmit}
              className="rl-field"
              noValidate
              data-testid="rl-create-invite-form"
            >
              <div className="rl-field">
                <label htmlFor={hintInputId} className="rl-label">
                  {strings.inviteeHintLabel}
                </label>
                <input
                  id={hintInputId}
                  type="text"
                  className="rl-input"
                  value={inviteeHint}
                  disabled={busy || !canManageMembers}
                  aria-invalid={inviteError ? "true" : "false"}
                  aria-describedby={inviteError ? inviteErrId : undefined}
                  onChange={(e) => {
                    setInviteeHint(e.currentTarget.value);
                    if (inviteError) setInviteError(null);
                  }}
                  data-testid="rl-invite-hint-input"
                />
              </div>
              <div className="rl-field">
                <label htmlFor={roleSelectId} className="rl-label">
                  {strings.inviteRoleLabel}
                </label>
                <select
                  id={roleSelectId}
                  className="rl-toolbar-select"
                  value={inviteRole}
                  disabled={busy || !canManageMembers}
                  onChange={(e) => setInviteRole(e.currentTarget.value as WorkspaceRole)}
                  data-testid="rl-invite-role-select"
                >
                  {INVITE_ROLES.map((role) => (
                    <option key={role} value={role}>
                      {strings.roleLabel(role)}
                    </option>
                  ))}
                </select>
              </div>
              {inviteError ? (
                <div id={inviteErrId} className="rl-error-box" role="alert">
                  {inviteError}
                </div>
              ) : null}
              <div className="rl-actions-row">
                <ActionButton
                  type="submit"
                  variant="primary"
                  label={strings.createInviteBtn}
                  disabled={busy || !canManageMembers}
                  testId="rl-create-invite-submit"
                />
              </div>
            </form>
          )}

          {lastCreatedInvite ? (
            <div className="rl-invite-token-box" data-testid="rl-created-invite-token-box">
              <span className="rl-label">{strings.oneTimeTokenWarning}</span>
              <code className="rl-invite-token-code" data-testid="rl-created-invite-token">
                {lastCreatedInvite.invite_token}
              </code>
            </div>
          ) : null}
        </div>

        <div className="rl-field" style={{ gap: "0.5rem" }}>
          <h3 className="rl-meta-label">{strings.invitationsHeading(invitations.length)}</h3>
          {invitations.length === 0 ? (
            <p className="rl-status-line">{strings.invitationsEmpty}</p>
          ) : (
            <ul className="rl-team-list" data-testid="rl-invitations-list">
              {invitations.map((inv) => {
                const tone =
                  inv.status === "accepted"
                    ? "success"
                    : inv.status === "pending"
                      ? "attention"
                      : inv.status === "revoked"
                        ? "danger"
                        : "neutral";
                return (
                  <li key={inv.id} className="rl-team-row" data-testid={`rl-invite-row-${inv.id}`}>
                    <div className="rl-team-row-info">
                      <span className="rl-comment-author">{inv.invitee_hint}</span>
                      <span className="rl-meta-value">
                        {inv.id} · {strings.roleLabel(inv.role)}
                      </span>
                    </div>
                    <div className="rl-actions-row">
                      <StatusBadge
                        status="queued"
                        customTone={tone}
                        customLabel={strings.inviteStatusLabel(inv.status)}
                      />
                      {inv.status === "pending" && canManageMembers ? (
                        <ActionButton
                          variant="danger"
                          label={strings.revokeInviteBtn}
                          disabled={busy}
                          onClick={() => onRevokeInvitation(inv.id)}
                          testId={`rl-revoke-invite-${inv.id}`}
                        />
                      ) : null}
                    </div>
                  </li>
                );
              })}
            </ul>
          )}
        </div>
      </div>
    </article>
  );
}
