import { wrap } from "@reatom/core";
import { reatomComponent } from "@reatom/react";
import type { ReactNode } from "react";
import type { Locale } from "@/common/i18n";
import { getTeamStrings } from "@/common/team-i18n";
import { TeamConnectionCard } from "@/common/ui/TeamConnectionCard";
import { TeamMembersInvitesPanel } from "@/common/ui/TeamMembersInvitesPanel";
import { TeamRunnersDraftsPanel } from "@/common/ui/TeamRunnersDraftsPanel";
import type { TeamCollabModel } from "./model";

export interface TeamWorkspaceViewProps {
  readonly model: TeamCollabModel;
  readonly locale: Locale;
}

export const TeamWorkspaceView = reatomComponent<TeamWorkspaceViewProps>(
  ({ model, locale }): ReactNode => {
    const strings = getTeamStrings(locale);
    const status = model.teamStatusResource.data();
    const wid = model.effectiveWorkspaceId();
    const members = model.members();
    const invitations = model.invitationsResource.data();
    const invitationsForbidden = model.invitationsForbidden();
    const lastCreatedInvite = model.lastCreatedInvite();
    const canManage = model.canManageMembers();
    const runnerStatus = model.runnerStatusResource.data();
    const runners = model.runners();
    const drafts = model.drafts();
    const busy = model.actionPending() !== null;
    const actionError = model.actionError();

    return (
      <div className="rl-team-grid" data-testid="rl-team-workspace-view">
        <div className="rl-page-header">
          <div>
            <h1 className="rl-page-h1">{strings.teamHeading}</h1>
            <p className="rl-page-subtitle">{strings.teamSubtitle}</p>
          </div>
        </div>

        {actionError ? (
          <div className="rl-error-box" role="alert" data-testid="rl-team-action-error">
            {actionError}
          </div>
        ) : null}

        <TeamConnectionCard
          sessions={status.sessions}
          workspaces={status.workspaces}
          selectedWorkspaceId={wid}
          busy={busy}
          strings={strings}
          onSelectWorkspace={wrap((next) => model.selectWorkspace(next))}
          onLogin={wrap((input) => void model.loginTeam(input))}
          onAcceptInvitation={wrap((input) => void model.acceptInvitation(input))}
          onCreateWorkspace={wrap((input) => void model.createWorkspace(input))}
        />

        <TeamMembersInvitesPanel
          workspaceId={wid}
          members={members}
          invitations={invitations}
          invitationsForbidden={invitationsForbidden}
          lastCreatedInvite={lastCreatedInvite}
          canManageMembers={canManage}
          busy={busy}
          strings={strings}
          onUpdateMemberRole={wrap((uid, role) => void model.updateMemberRole(uid, role))}
          onCreateInvitation={wrap((input) => void model.createInvitation(input))}
          onRevokeInvitation={wrap((iid) => void model.revokeInvitation(iid))}
        />

        <TeamRunnersDraftsPanel
          workspaceId={wid}
          runnerStatus={runnerStatus}
          runners={runners}
          drafts={drafts}
          busy={busy}
          strings={strings}
          onRegisterRunner={wrap((name) => void model.registerRunner(name))}
          onRetryDraft={wrap((did, input) => void model.retryDraft(did, input))}
          onDeleteDraft={wrap((did) => void model.deleteDraft(did))}
        />
      </div>
    );
  },
  "TeamWorkspaceView",
);
