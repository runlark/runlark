import { action, atom, computed, sleep, wrap } from "@reatom/core";
import {
  resolveDisplayRunId,
  type DaemonEventItem,
  type DaemonQuestion,
  type DaemonSession,
  type RunlarkApiClient,
} from "@/common/runlark-types";

export type ConnectionState = "connecting" | "live" | "reconnecting" | "error";

export interface SessionCardModel {
  readonly sessionId: string;
  readonly session: ReturnType<typeof atom<DaemonSession | null>>;
  readonly questions: ReturnType<typeof atom<readonly DaemonQuestion[]>>;
  readonly events: ReturnType<typeof atom<readonly DaemonEventItem[]>>;
  readonly eventsRunId: ReturnType<typeof atom<string | null>>;
  readonly lastSeq: ReturnType<typeof computed<number | null>>;
  readonly activeQuestion: ReturnType<typeof computed<DaemonQuestion | null>>;
  readonly connectionState: ReturnType<typeof atom<ConnectionState>>;
  readonly loading: ReturnType<typeof atom<boolean>>;
  readonly actionPending: ReturnType<typeof atom<string | null>>;
  readonly actionError: ReturnType<typeof atom<string | null>>;
  readonly questionError: ReturnType<typeof atom<string | null>>;
  readonly streamError: ReturnType<typeof atom<string | null>>;
  readonly refresh: () => Promise<void>;
  readonly attachSession: (taskId: string, cwd: string) => Promise<void>;
  readonly sendPrompt: (prompt: string) => Promise<void>;
  readonly stopRun: () => Promise<void>;
  readonly resumeSession: () => Promise<void>;
  readonly archiveSession: () => Promise<void>;
  readonly answerQuestion: (questionId: string, answer: string) => Promise<void>;
  readonly startLiveSync: (opts?: {
    readonly waitMs?: number;
    readonly retryDelayMs?: number;
  }) => () => void;
}

function mergeEvents(
  existing: readonly DaemonEventItem[],
  incoming: readonly DaemonEventItem[],
  targetRunId: string | null,
  previousRunId: string | null,
): readonly DaemonEventItem[] {
  if (incoming.length === 0) {
    return targetRunId !== previousRunId ? [] : existing;
  }
  const base = targetRunId !== previousRunId ? [] : [...existing];
  const seen = new Set(
    base.map((item) => `${item.run_id ?? targetRunId ?? "run"}:${String(item.seq)}`),
  );
  for (const item of incoming) {
    const key = `${item.run_id ?? targetRunId ?? "run"}:${String(item.seq)}`;
    if (!seen.has(key)) {
      seen.add(key);
      base.push(item);
    }
  }
  base.sort((a, b) => a.seq - b.seq);
  return base;
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export function reatomSessionCardModel(api: RunlarkApiClient, sessionId: string): SessionCardModel {
  const prefix = `sessionCard.${sessionId}`;
  const session = atom<DaemonSession | null>(null, `${prefix}.session`);
  const questions = atom<readonly DaemonQuestion[]>([], `${prefix}.questions`);
  const events = atom<readonly DaemonEventItem[]>([], `${prefix}.events`);
  const eventsRunId = atom<string | null>(null, `${prefix}.eventsRunId`);
  const connectionState = atom<ConnectionState>("connecting", `${prefix}.connectionState`);
  const loading = atom<boolean>(true, `${prefix}.loading`);
  const actionPending = atom<string | null>(null, `${prefix}.actionPending`);
  const actionError = atom<string | null>(null, `${prefix}.actionError`);
  const questionError = atom<string | null>(null, `${prefix}.questionError`);
  const streamError = atom<string | null>(null, `${prefix}.streamError`);

  const lastSeq = computed<number | null>(() => {
    const list = events();
    const last = list.at(-1);
    return last ? last.seq : null;
  }, `${prefix}.lastSeq`);

  const activeQuestion = computed<DaemonQuestion | null>(() => {
    const pendingFromList = questions().find((q) => q.status === "pending");
    if (pendingFromList) {
      return pendingFromList;
    }
    const fromSession = session()?.pending_question;
    return fromSession && fromSession.status === "pending" ? fromSession : null;
  }, `${prefix}.activeQuestion`);

  const syncOnce = action(async (signal?: AbortSignal) => {
    const loadedSession = await wrap(api.getSession(sessionId, signal));
    if (signal?.aborted) {
      return;
    }
    session.set(loadedSession);
    if (!loadedSession) {
      questions.set([]);
      events.set([]);
      eventsRunId.set(null);
      loading.set(false);
      connectionState.set("live");
      streamError.set(null);
      return;
    }

    const loadedQuestions = await wrap(api.listSessionQuestions(sessionId, signal));
    if (signal?.aborted) {
      return;
    }
    questions.set(loadedQuestions);

    const targetRunId = resolveDisplayRunId(loadedSession);
    const prevRunId = eventsRunId();
    const afterSeq = targetRunId === prevRunId ? (lastSeq() ?? undefined) : undefined;
    if (targetRunId) {
      const fetchedEvents = await wrap(
        api.getSessionEvents(sessionId, {
          runId: targetRunId,
          ...(afterSeq !== undefined ? { afterSeq } : {}),
          ...(signal ? { signal } : {}),
        }),
      );
      if (signal?.aborted) {
        return;
      }
      const merged = mergeEvents(events(), fetchedEvents, targetRunId, prevRunId);
      events.set(merged);
      eventsRunId.set(targetRunId);
    } else {
      events.set([]);
      eventsRunId.set(null);
    }

    loading.set(false);
    connectionState.set("live");
    streamError.set(null);
  }, `${prefix}.syncOnce`);

  const refresh = action(async () => {
    try {
      await wrap(syncOnce());
    } catch (error: unknown) {
      loading.set(false);
      connectionState.set("error");
      streamError.set(errorMessage(error));
    }
  }, `${prefix}.refresh`);

  const attachSession = action(async (taskId: string, cwd: string) => {
    actionPending.set("attach");
    actionError.set(null);
    try {
      const ensured = await wrap(api.ensureSession({ sessionId, taskId, cwd }));
      session.set(ensured);
      await wrap(syncOnce());
    } catch (error: unknown) {
      actionError.set(errorMessage(error));
    } finally {
      actionPending.set(null);
    }
  }, `${prefix}.attachSession`);

  const sendPrompt = action(async (prompt: string) => {
    const current = session();
    if (!current) {
      return;
    }
    actionPending.set("send");
    actionError.set(null);
    try {
      await wrap(
        api.sendPrompt({
          sessionId,
          prompt,
          expectedRevision: current.current_revision,
          activeRunId: current.active_run_id,
        }),
      );
      await wrap(syncOnce());
    } catch (error: unknown) {
      actionError.set(errorMessage(error));
    } finally {
      actionPending.set(null);
    }
  }, `${prefix}.sendPrompt`);

  const stopRun = action(async () => {
    const current = session();
    if (!current?.active_run_id) {
      return;
    }
    actionPending.set("stop");
    actionError.set(null);
    try {
      await wrap(
        api.stopRun({
          sessionId,
          runId: current.active_run_id,
          expectedRevision: current.current_revision,
        }),
      );
      await wrap(syncOnce());
    } catch (error: unknown) {
      actionError.set(errorMessage(error));
    } finally {
      actionPending.set(null);
    }
  }, `${prefix}.stopRun`);

  const resumeSession = action(async () => {
    const current = session();
    const runId = current ? resolveDisplayRunId(current) : null;
    if (!current || !runId) {
      return;
    }
    actionPending.set("resume");
    actionError.set(null);
    try {
      const seq = lastSeq();
      await wrap(
        api.resumeSession({
          sessionId,
          runId,
          expectedRevision: current.current_revision,
          ...(seq !== null ? { afterSeq: seq } : {}),
        }),
      );
      await wrap(syncOnce());
    } catch (error: unknown) {
      actionError.set(errorMessage(error));
    } finally {
      actionPending.set(null);
    }
  }, `${prefix}.resumeSession`);

  const archiveSession = action(async () => {
    const current = session();
    if (!current) {
      return;
    }
    actionPending.set("archive");
    actionError.set(null);
    try {
      await wrap(
        api.archiveSession({
          sessionId,
          runId: resolveDisplayRunId(current),
          expectedRevision: current.current_revision,
        }),
      );
      await wrap(syncOnce());
    } catch (error: unknown) {
      actionError.set(errorMessage(error));
    } finally {
      actionPending.set(null);
    }
  }, `${prefix}.archiveSession`);

  const answerQuestion = action(async (questionId: string, answer: string) => {
    const current = session();
    if (!current) {
      return;
    }
    actionPending.set("answer");
    questionError.set(null);
    try {
      await wrap(
        api.answerQuestion({
          sessionId,
          questionId,
          answer,
          expectedRevision: current.current_revision,
        }),
      );
      await wrap(syncOnce());
    } catch (error: unknown) {
      questionError.set(errorMessage(error));
    } finally {
      actionPending.set(null);
    }
  }, `${prefix}.answerQuestion`);

  const startLiveSync = (
    opts: { readonly waitMs?: number; readonly retryDelayMs?: number } = {},
  ): (() => void) => {
    const waitMs = opts.waitMs ?? 10_000;
    const retryDelayMs = opts.retryDelayMs ?? 1_500;
    const controller = new AbortController();
    const { signal } = controller;

    const runLoop = action(async () => {
      while (!signal.aborted) {
        try {
          if (connectionState() === "error") {
            connectionState.set("reconnecting");
          }
          await wrap(syncOnce(signal));
          if (signal.aborted) {
            break;
          }

          const current = session();
          const currentRunId = current ? resolveDisplayRunId(current) : null;
          const seq = currentRunId && currentRunId === eventsRunId() ? lastSeq() : null;
          const updates = await wrap(
            api.getSessionEvents(sessionId, {
              ...(seq !== null ? { afterSeq: seq } : {}),
              waitMs,
              signal,
            }),
          );
          if (signal.aborted) {
            break;
          }
          if (updates.length > 0) {
            await wrap(syncOnce(signal));
          } else if (current === null) {
            await wrap(sleep(retryDelayMs));
          }
        } catch (error: unknown) {
          if (signal.aborted) {
            break;
          }
          loading.set(false);
          connectionState.set("error");
          streamError.set(errorMessage(error));
          try {
            await wrap(sleep(retryDelayMs));
          } catch {
            break;
          }
        }
      }
    }, `${prefix}.liveLoop`);

    void runLoop();

    return () => {
      controller.abort();
    };
  };

  return {
    sessionId,
    session,
    questions,
    events,
    eventsRunId,
    lastSeq,
    activeQuestion,
    connectionState,
    loading,
    actionPending,
    actionError,
    questionError,
    streamError,
    refresh,
    attachSession,
    sendPrompt,
    stopRun,
    resumeSession,
    archiveSession,
    answerQuestion,
    startLiveSync,
  };
}
