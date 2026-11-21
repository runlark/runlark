import type { ReactNode } from "react";
import type { ArtifactItem } from "../board-types";
import type { RunlarkStrings } from "../i18n";
import type { TeamStrings } from "../team-i18n";
import type { VerifiedArtifactInspection } from "../team-types";
import { ActionButton } from "./ActionButton";
import { StatusBadge } from "./StatusBadge";
import { ShieldCheckIconSvg } from "./SvgIcons";

export interface CasArtifactEvidencePanelProps {
  readonly artifacts: readonly ArtifactItem[];
  readonly inspections: Readonly<Record<string, VerifiedArtifactInspection>>;
  readonly inspectingDigest: string | null;
  readonly strings: RunlarkStrings;
  readonly teamStrings: TeamStrings;
  readonly onInspectArtifact: (digest: string) => void;
  readonly onSyncWorkspace?: () => void;
}

export function CasArtifactEvidencePanel({
  artifacts,
  inspections,
  inspectingDigest,
  strings,
  teamStrings,
  onInspectArtifact,
  onSyncWorkspace,
}: CasArtifactEvidencePanelProps): ReactNode {
  return (
    <article className="rl-card" data-testid="rl-cas-artifacts-panel">
      <header className="rl-card-header">
        <h3 className="rl-section-heading">{strings.casArtifactsHeading(artifacts.length)}</h3>
        <ShieldCheckIconSvg />
      </header>

      {artifacts.length === 0 ? (
        <p className="rl-status-line">{strings.artifactsEmpty}</p>
      ) : (
        <ul className="rl-team-list" data-testid="rl-cas-artifacts-list">
          {artifacts.map((art) => {
            const inspection = inspections[art.digest_sha256];
            const isLoading = inspectingDigest === art.digest_sha256;

            return (
              <li
                key={`${art.label}-${art.digest_sha256}`}
                className="rl-artifact-row"
                data-testid={`rl-cas-artifact-${art.label}`}
              >
                <div className="rl-artifact-head">
                  <div className="rl-sync-strip-group">
                    <span className="rl-event-type">{art.label}</span>
                    <span className="rl-badge rl-badge-neutral rl-mono">{art.kind}</span>
                    <span className="rl-figure">{String(art.size_bytes)} B</span>
                  </div>
                  <div className="rl-actions-row">
                    {inspection?.status === "verified" ? (
                      <StatusBadge
                        status="completed"
                        customTone="success"
                        customLabel={teamStrings.casVerifiedBadge}
                      />
                    ) : inspection?.status === "missing" ? (
                      <StatusBadge
                        status="unknown"
                        customTone="unknown"
                        customLabel={teamStrings.casMissingBadge}
                      />
                    ) : inspection?.status === "corrupted" ? (
                      <StatusBadge
                        status="failed"
                        customTone="danger"
                        customLabel={teamStrings.casCorruptedBadge}
                      />
                    ) : null}

                    <ActionButton
                      variant="secondary"
                      label={isLoading ? teamStrings.casInspectingBtn : teamStrings.casInspectBtn}
                      disabled={isLoading}
                      onClick={() => onInspectArtifact(art.digest_sha256)}
                      testId={`rl-cas-inspect-${art.label}`}
                    />
                  </div>
                </div>

                <p className="rl-meta-value" data-testid={`rl-cas-digest-${art.label}`}>
                  sha256:{art.digest_sha256}
                </p>

                {inspection?.status === "verified" ? (
                  <div className="rl-field" style={{ gap: "0.25rem" }}>
                    <pre
                      className="rl-artifact-preview"
                      data-testid={`rl-cas-preview-${art.label}`}
                    >
                      {inspection.textPreview}
                    </pre>
                    {inspection.truncated ? (
                      <span className="rl-meta-label">{teamStrings.casTruncatedNotice}</span>
                    ) : null}
                  </div>
                ) : null}

                {inspection?.status === "missing" ? (
                  <div className="rl-actions-row">
                    <span className="rl-status-line">{inspection.message}</span>
                    {onSyncWorkspace ? (
                      <ActionButton
                        variant="secondary"
                        label={teamStrings.syncNowBtn}
                        onClick={onSyncWorkspace}
                      />
                    ) : null}
                  </div>
                ) : null}

                {inspection?.status === "corrupted" || inspection?.status === "error" ? (
                  <div
                    className="rl-error-box"
                    role="alert"
                    data-testid={`rl-cas-error-${art.label}`}
                  >
                    {inspection.message}
                  </div>
                ) : null}
              </li>
            );
          })}
        </ul>
      )}
    </article>
  );
}
