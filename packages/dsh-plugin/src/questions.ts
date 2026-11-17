import { randomUUID } from "node:crypto";
import type { Context } from "@deepseek-ai/cordis";
import { brandString } from "@deepseek-ai/dsh-brand";
import { ToolCallId } from "@deepseek-ai/dsh-llm";
import { SessionId as brandSessionId } from "@deepseek-ai/dsh-session";
import type {} from "@deepseek-ai/dsh-user-questions";
import type { DaemonClient } from "./daemon_client.ts";
import type { RunlarkSessionInterceptor } from "./interceptor.ts";
import type { AskUserQuestionAnswer, AskUserQuestionItem, SessionRequestId } from "./types.ts";

interface PendingQuestionWaiter {
  callId: string;
  sessionId: string;
  questions: readonly AskUserQuestionItem[];
  resolve: (answer: AskUserQuestionAnswer) => void;
  reject: (reason?: unknown) => void;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function parseAskUserQuestions(
  rawArgs: string,
): { prompt: string; options: string[]; questions: AskUserQuestionItem[] } | null {
  try {
    const parsed = JSON.parse(rawArgs) as unknown;
    if (!isRecord(parsed) || !Array.isArray(parsed.questions) || parsed.questions.length === 0) {
      return null;
    }
    const questions: AskUserQuestionItem[] = [];
    const prompts: string[] = [];
    const options: string[] = [];
    for (const item of parsed.questions) {
      if (!isRecord(item) || typeof item.id !== "string" || typeof item.question !== "string") {
        continue;
      }
      prompts.push(item.question);
      const itemOpts: { label: string; description?: string }[] = [];
      if (Array.isArray(item.options)) {
        for (const opt of item.options) {
          if (isRecord(opt) && typeof opt.label === "string") {
            options.push(opt.label);
            itemOpts.push({
              label: opt.label,
              ...(typeof opt.description === "string" ? { description: opt.description } : {}),
            });
          }
        }
      }
      questions.push({
        id: item.id,
        question: item.question,
        ...(itemOpts.length > 0 ? { options: itemOpts } : {}),
        ...(item.multi_select === true ? { multiSelect: true } : {}),
      });
    }
    if (questions.length === 0) return null;
    return { prompt: prompts.join("\n"), options, questions };
  } catch {
    return null;
  }
}

export function formatAnswerText(answer: AskUserQuestionAnswer): string {
  if (answer.answers.length === 1) {
    const item = answer.answers[0];
    if (item.custom !== undefined && item.custom.length > 0) return item.custom;
    if (item.selected.length > 0) return item.selected.join(", ");
  }
  return JSON.stringify(answer);
}

export function buildStructuredAnswer(
  questions: readonly AskUserQuestionItem[],
  answerText: string,
): AskUserQuestionAnswer {
  try {
    const parsed = JSON.parse(answerText) as unknown;
    if (isRecord(parsed) && Array.isArray(parsed.answers)) {
      return parsed as unknown as AskUserQuestionAnswer;
    }
  } catch {
    // Plain text answers map to matching option labels or custom text.
  }
  return {
    answers: questions.map((q) => {
      const matched = q.options?.find((opt) => opt.label === answerText);
      return matched
        ? { id: q.id, selected: [matched.label] }
        : { id: q.id, selected: [], custom: answerText };
    }),
  };
}

export class QuestionCoordinator {
  private readonly ctx: Context;
  private readonly client: DaemonClient;
  private readonly interceptor: RunlarkSessionInterceptor;
  private readonly latestQuestionBySession = new Map<
    string,
    { callId: string; questions: readonly AskUserQuestionItem[] }
  >();
  private readonly pendingQuestionWaiters = new Map<string, PendingQuestionWaiter>();
  private readonly rustAnsweredQuestions = new Set<string>();
  private readonly deliveredExternalAnswers = new Set<string>();

  constructor(ctx: Context, client: DaemonClient, interceptor: RunlarkSessionInterceptor) {
    this.ctx = ctx;
    this.client = client;
    this.interceptor = interceptor;
  }

  trackToolCall(sessionId: string, callId: string, rawArgs: string): void {
    const parsed = parseAskUserQuestions(rawArgs);
    if (parsed) {
      this.latestQuestionBySession.set(sessionId, {
        callId,
        questions: parsed.questions,
      });
    }
  }

  clearSession(sessionId: string): void {
    this.latestQuestionBySession.delete(sessionId);
  }

  registerWaterfallListener(): void {
    this.ctx.inject(["userQuestions"], (qCtx) => {
      qCtx.on(
        "user-questions/request",
        async (request, next) => {
          const agent = request.agent;
          if (!agent || !this.interceptor.handles(agent.id)) {
            return await next();
          }
          const sessionId = agent.id;
          const tracked = this.latestQuestionBySession.get(sessionId);
          const callId = request.wait?.callId ?? tracked?.callId ?? `q_${randomUUID()}`;
          const runId = this.interceptor.getActiveRunId(sessionId) ?? `run_${sessionId}`;

          if (!tracked || tracked.callId !== callId) {
            const prompt = request.questions.map((q) => q.question).join("\n");
            const options = request.questions.flatMap((q) => q.options?.map((o) => o.label) ?? []);
            this.latestQuestionBySession.set(sessionId, { callId, questions: request.questions });
            await this.client
              .postDshEvent({
                UserQuestionAsked: {
                  session_id: sessionId,
                  run_id: runId,
                  question_id: callId,
                  prompt,
                  options,
                },
              })
              .catch(() => {});
          }

          let settled = false;
          const rustAnswerPromise = new Promise<AskUserQuestionAnswer>((resolve, reject) => {
            const waiter: PendingQuestionWaiter = {
              callId,
              sessionId,
              questions: request.questions,
              resolve: (ans) => {
                settled = true;
                resolve(ans);
              },
              reject: (err) => {
                settled = true;
                reject(err);
              },
            };
            this.pendingQuestionWaiters.set(callId, waiter);
            if (request.signal) {
              if (request.signal.aborted) {
                this.pendingQuestionWaiters.delete(callId);
                waiter.reject(request.signal.reason);
                return;
              }
              request.signal.addEventListener(
                "abort",
                () => {
                  if (this.pendingQuestionWaiters.delete(callId)) {
                    waiter.reject(request.signal?.reason);
                  }
                },
                { once: true },
              );
            }
          });

          const browserPromise = next().catch(async (err: unknown) => {
            if (request.signal?.aborted) {
              throw err;
            }
            // No browser answerer accepted the request; keep waiting for an external Rust/UI answer.
            return await rustAnswerPromise;
          });

          try {
            const answer = await Promise.race([browserPromise, rustAnswerPromise]);
            if (!this.rustAnsweredQuestions.delete(callId)) {
              const answerText = formatAnswerText(answer);
              await this.client
                .answerQuestion(callId, {
                  command_id: `dsh_ui_ans_${callId}`,
                  answer: answerText,
                })
                .catch(() => {});
            }
            return answer;
          } finally {
            if (!settled) {
              this.pendingQuestionWaiters.delete(callId);
            }
          }
        },
        { prepend: true, global: true },
      );
    });
  }

  private findQuestionsInEvents(
    events: readonly { type: string; data?: unknown }[],
    callId: string,
  ): readonly AskUserQuestionItem[] | null {
    for (let i = events.length - 1; i >= 0; i -= 1) {
      const ev = events[i];
      if (
        ev.type === "tool/call" &&
        isRecord(ev.data) &&
        ev.data.callId === callId &&
        typeof ev.data.arguments === "string"
      ) {
        const parsed = parseAskUserQuestions(ev.data.arguments);
        if (parsed) {
          return parsed.questions;
        }
      }
    }
    return null;
  }

  async applyExternalAnswer(sessionId: string, callId: string, answerText: string): Promise<void> {
    if (this.deliveredExternalAnswers.has(callId)) {
      return;
    }
    this.deliveredExternalAnswers.add(callId);
    if (this.deliveredExternalAnswers.size > 1000) {
      const oldest = this.deliveredExternalAnswers.values().next().value;
      if (oldest !== undefined) {
        this.deliveredExternalAnswers.delete(oldest);
      }
    }
    this.rustAnsweredQuestions.add(callId);
    const waiter = this.pendingQuestionWaiters.get(callId);
    if (waiter) {
      this.pendingQuestionWaiters.delete(callId);
      waiter.resolve(buildStructuredAnswer(waiter.questions, answerText));
      return;
    }

    const sid = brandSessionId(sessionId);
    let agent = this.ctx.agents.get(sid);
    if (!agent) {
      const resolved = await this.ctx.sessionController.resolveAgent(sid).catch(() => null);
      if (resolved && "agent" in resolved) {
        agent = resolved.agent;
      }
    }
    if (!agent) {
      return;
    }

    const tracked = this.latestQuestionBySession.get(sessionId);
    const questions = (tracked?.callId === callId ? tracked.questions : null) ??
      this.findQuestionsInEvents(agent.session.snapshotEvents(), callId) ?? [
        { id: callId, question: callId },
      ];

    const userQuestions = this.ctx.get("userQuestions");
    let accepted = false;
    if (userQuestions) {
      try {
        accepted = userQuestions.answer(
          agent,
          ToolCallId(callId),
          buildStructuredAnswer(questions, answerText),
        );
      } catch {
        accepted = false;
      }
    }

    if (!accepted) {
      const reqId = brandString<SessionRequestId>(`ans_${callId}_${randomUUID()}`);
      this.interceptor.markDispatchedCommand(reqId);
      await this.ctx.sessionController.prompt(
        {
          requestId: reqId,
          sessionId: sid,
          mode: "queue",
          content: [{ type: "text", text: `Answer to question ${callId}: ${answerText}` }],
        },
        new AbortController().signal,
      );
    }
  }
}
