import type { Context } from "@deepseek-ai/cordis";
import type {} from "@deepseek-ai/dsh-tools";
import { DaemonBoardClient } from "./daemon_board_client.ts";

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function registerRunlarkChatTool(ctx: Context, boardClient: DaemonBoardClient): void {
  ctx.inject(["tools"], (toolCtx) => {
    toolCtx.tools.register({
      name: "runlark_chat_command",
      description:
        "Execute a Runlark workflow slash command (/task, /fix, /plan, /run, /stop, /resume, /accept, /reject, /answer, /pr, /status, /board, /budget) via the local Runlark daemon.",
      parameters: {
        type: "object",
        properties: {
          input: {
            type: "string",
            description: "Raw slash command or workflow prompt to dispatch.",
          },
          project_id: { type: "string" },
          session_id: { type: "string" },
          run_id: { type: "string" },
        },
        required: ["input"],
        additionalProperties: false,
      },
      output: {
        schema: {
          type: "object",
          properties: {
            command: { type: "string" },
            summary: { type: "string" },
            data: { type: "object" },
          },
          required: ["command", "summary"],
          additionalProperties: true,
        },
        render: (_args, value) => {
          const text =
            isRecord(value) && typeof value.summary === "string"
              ? value.summary
              : JSON.stringify(value);
          return [{ type: "text", text }];
        },
      },
      execute: async (rawArgs, exec) => {
        if (!isRecord(rawArgs) || typeof rawArgs.input !== "string" || !rawArgs.input.trim()) {
          throw new Error("runlark_chat_command requires a non-empty string `input`");
        }
        return await boardClient.dispatchChat(
          {
            input: rawArgs.input,
            ...(typeof rawArgs.project_id === "string" ? { project_id: rawArgs.project_id } : {}),
            ...(typeof rawArgs.session_id === "string" ? { session_id: rawArgs.session_id } : {}),
            ...(typeof rawArgs.run_id === "string" ? { run_id: rawArgs.run_id } : {}),
          },
          exec.signal,
        );
      },
    });
  });
}
