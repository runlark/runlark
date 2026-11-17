import type { DaemonBoardClient, RecordAttemptPayload } from "./daemon_board_client.ts";
import type { SessionEvent } from "./types.ts";

export interface ExtractedTurnUsage {
  costMicroUsd: number | null;
  unknownCostReason: string | null;
  inputTokens?: number;
  outputTokens?: number;
  totalTokens?: number;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function extractNumericCostMicroUsd(obj: Record<string, unknown>): number | null {
  if (typeof obj.costMicroUsd === "number" && Number.isFinite(obj.costMicroUsd)) {
    return Math.max(0, Math.round(obj.costMicroUsd));
  }
  if (typeof obj.cost_micro_usd === "number" && Number.isFinite(obj.cost_micro_usd)) {
    return Math.max(0, Math.round(obj.cost_micro_usd));
  }
  if (typeof obj.costUsd === "number" && Number.isFinite(obj.costUsd)) {
    return Math.max(0, Math.round(obj.costUsd * 1_000_000));
  }
  if (typeof obj.cost_usd === "number" && Number.isFinite(obj.cost_usd)) {
    return Math.max(0, Math.round(obj.cost_usd * 1_000_000));
  }
  return null;
}

/**
 * Extracts explicit attempt cost (or explicit `null` UnknownCost — NEVER `0`) from a DSH
 * `turn/end` payload and optional preceding `assistant/message` usage.
 */
export function extractAttemptCostFromTurnEnd(
  turnEndData: unknown,
  lastUsageData?: unknown,
): ExtractedTurnUsage {
  const endObj = isRecord(turnEndData) ? turnEndData : {};
  const usageCandidate = isRecord(endObj.usage)
    ? endObj.usage
    : isRecord(lastUsageData)
      ? lastUsageData
      : undefined;

  const explicitCost =
    extractNumericCostMicroUsd(endObj) ??
    (usageCandidate ? extractNumericCostMicroUsd(usageCandidate) : null);

  const inputTokens =
    usageCandidate && typeof usageCandidate.inputTokens === "number"
      ? usageCandidate.inputTokens
      : undefined;
  const outputTokens =
    usageCandidate && typeof usageCandidate.outputTokens === "number"
      ? usageCandidate.outputTokens
      : undefined;
  const totalTokens =
    usageCandidate && typeof usageCandidate.totalTokens === "number"
      ? usageCandidate.totalTokens
      : undefined;

  if (explicitCost !== null) {
    return {
      costMicroUsd: explicitCost,
      unknownCostReason: null,
      inputTokens,
      outputTokens,
      totalTokens,
    };
  }

  const reason =
    inputTokens !== undefined || outputTokens !== undefined
      ? `provider reported tokens (in=${inputTokens ?? 0}, out=${outputTokens ?? 0}) without USD pricing`
      : "DSH turn/end omitted token and cost telemetry";
  return {
    costMicroUsd: null,
    unknownCostReason: reason,
    inputTokens,
    outputTokens,
    totalTokens,
  };
}

export class UsageBridge {
  private readonly boardClient: DaemonBoardClient;
  private readonly lastUsageBySession = new Map<string, unknown>();

  constructor(boardClient: DaemonBoardClient) {
    this.boardClient = boardClient;
  }

  observeEvent(sessionId: string, event: SessionEvent): void {
    if (event.type === "assistant/message" && isRecord(event.data) && "usage" in event.data) {
      this.lastUsageBySession.set(sessionId, event.data.usage);
    }
  }

  async recordTurnEndUsage(
    sessionId: string,
    runId: string,
    event: SessionEvent,
  ): Promise<RecordAttemptPayload> {
    const lastUsage = this.lastUsageBySession.get(sessionId);
    this.lastUsageBySession.delete(sessionId);
    const extracted = extractAttemptCostFromTurnEnd(event.data, lastUsage);

    const dataObj = isRecord(event.data) ? (event.data as Record<string, unknown>) : {};
    const reasonKind =
      isRecord(dataObj.reason) && typeof dataObj.reason.kind === "string"
        ? dataObj.reason.kind
        : "completed";
    const outcome =
      reasonKind === "error"
        ? { status: "failed", error: "DSH turn failed" }
        : reasonKind === "aborted" || reasonKind === "blocked" || reasonKind === "interrupted"
          ? { status: "stopped" }
          : { status: "succeeded" };

    const payload: RecordAttemptPayload = {
      command_id: `dsh_usage_${runId}_${event.seq}`,
      attempt_id: `att_dsh_${runId}_${event.seq}`,
      workflow_stage: "implementation",
      outcome,
      cost_micro_usd: extracted.costMicroUsd,
      ...(extracted.unknownCostReason ? { unknown_cost_reason: extracted.unknownCostReason } : {}),
    };

    await this.boardClient.recordRunAttempt(runId, payload);
    return payload;
  }
}
