export type MessageSource = "user" | "agent" | "system";

export interface SessionPromptRequest {
  sessionId: string;
  prompt: string;
  runId?: string;
}

export interface SessionPromptValue {
  turnId: string;
}

export interface SessionCancelRequest {
  sessionId: string;
  reason?: string;
}

export interface SessionCancelValue {
  cancelled: boolean;
}

export interface SessionCommandInterceptor {
  handles(sessionId: string): boolean;
  prompt?(
    request: SessionPromptRequest,
    source: MessageSource,
    next: () => Promise<SessionPromptValue>,
  ): Promise<SessionPromptValue>;
  cancel?(
    request: SessionCancelRequest,
    next: () => SessionCancelValue,
  ): Promise<SessionCancelValue> | SessionCancelValue;
}

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

export type SessionCommand =
  | { type: "SendPrompt"; payload: SendPromptPayload }
  | { type: "StopRun"; payload: StopRunPayload }
  | { type: "ResumeSession"; payload: ResumeSessionPayload }
  | { type: "ChangeCwd"; payload: ChangeCwdPayload }
  | { type: "ArchiveSession"; payload: ArchiveSessionPayload };

export interface SubmitCommandResult {
  ok: boolean;
  command_id?: string;
  status?: string;
  error?: string;
}

export interface DaemonEventItem {
  seq: number;
  event_type: string;
  payload: unknown;
  created_at: number;
}
