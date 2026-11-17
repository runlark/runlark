import type {
  PromptContentPart,
  SessionCancelRequest,
  SessionCancelValue,
  SessionCommandInterceptor,
  SessionCreateRequest,
  SessionCreateValue,
  SessionFollowFrame,
  SessionFollowRequest,
  SessionPromptRequest,
  SessionPromptValue,
  SessionRequestId,
  SessionWireEvent,
} from "@deepseek-ai/dsh-api-session-controller";
import type { MessageSource } from "@deepseek-ai/dsh-llm";
import type { SessionEvent, SessionId } from "@deepseek-ai/dsh-session";
import type { AskUserQuestionAnswer, AskUserQuestionItem } from "@deepseek-ai/dsh-user-questions";

export type {
  AskUserQuestionAnswer,
  AskUserQuestionItem,
  MessageSource,
  PromptContentPart,
  SessionCancelRequest,
  SessionCancelValue,
  SessionCommandInterceptor,
  SessionCreateRequest,
  SessionCreateValue,
  SessionEvent,
  SessionFollowFrame,
  SessionFollowRequest,
  SessionId,
  SessionPromptRequest,
  SessionPromptValue,
  SessionRequestId,
  SessionWireEvent,
};

export interface SendPromptPayload {
  command_id: string;
  session_id: string;
  run_id: string;
  prompt: string;
  expected_revision: number;
  created_at: number;
}

export interface StopRunPayload {
  command_id: string;
  session_id: string;
  run_id: string;
  reason?: string;
  expected_revision: number;
  created_at: number;
}

export interface ResumeSessionPayload {
  command_id: string;
  session_id: string;
  run_id: string;
  after_seq?: number;
  expected_revision: number;
  created_at: number;
}

export interface ChangeCwdPayload {
  command_id: string;
  session_id: string;
  run_id: string;
  new_cwd: string;
  expected_revision: number;
  created_at: number;
}

export interface ArchiveSessionPayload {
  command_id: string;
  session_id: string;
  run_id: string;
  expected_revision: number;
  created_at: number;
}

export interface AnswerQuestionPayload {
  command_id: string;
  session_id: string;
  run_id: string;
  question_id: string;
  answer: string;
  expected_revision: number;
  created_at: number;
}

export type SessionCommand =
  | { type: "SendPrompt"; payload: SendPromptPayload }
  | { type: "StopRun"; payload: StopRunPayload }
  | { type: "ResumeSession"; payload: ResumeSessionPayload }
  | { type: "ChangeCwd"; payload: ChangeCwdPayload }
  | { type: "ArchiveSession"; payload: ArchiveSessionPayload }
  | { type: "AnswerQuestion"; payload: AnswerQuestionPayload };

export type TurnEndStatus = "Completed" | "Stopped" | { Failed: { error: string } };

export type RuntimeEvent =
  | { TurnStarted: { session_id: string; run_id: string } }
  | {
      UserQuestionAsked: {
        session_id: string;
        run_id: string;
        question_id: string;
        prompt: string;
        options: string[];
      };
    }
  | {
      TurnEnded: {
        session_id: string;
        run_id: string;
        outcome: TurnEndStatus;
      };
    };

export interface DshWireEventPayload {
  type: string;
  seq: number;
  time: number;
  data: unknown;
}

export interface DshWireEventEnvelope {
  session_id: string;
  run_id?: string;
  event: DshWireEventPayload;
}

export interface DaemonQuestion {
  question_id: string;
  session_id: string;
  run_id: string;
  prompt: string;
  options: string[];
  status: string;
  answer: string | null;
  created_at: number;
  updated_at: number;
}

export interface DaemonSession {
  session_id: string;
  task_id: string;
  cwd: string;
  status: string;
  current_revision: number;
  active_run_id: string | null;
  active_run_status?: string | null;
  worktree_path?: string | null;
  pending_question?: DaemonQuestion | null;
  created_at: number;
  updated_at: number;
}

export interface CreateSessionRequest {
  session_id: string;
  task_id: string;
  cwd: string;
}

export interface SubmitCommandResult {
  ok: boolean;
  command_id?: string;
  status?: string;
  current_revision?: number;
  run_id?: string;
  run_status?: string;
  conflict?: boolean;
  error_code?: string;
  actual_revision?: number;
  error?: string;
}

export interface AnswerQuestionRequest {
  command_id?: string;
  answer: string;
  expected_revision?: number;
}

export interface AnswerQuestionResponse {
  command_id: string;
  status: string;
  question_id: string;
  current_revision: number;
  run_status?: string;
}

export interface DaemonEventItem {
  seq: number;
  event_type: string;
  payload: unknown;
  created_at: number;
}

export interface OutboxDispatchRequest {
  command_id: string;
  session_id: string;
  run_id: string;
  command: SessionCommand;
  worktree_path: string | null;
}
