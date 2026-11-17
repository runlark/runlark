import type { DaemonEventItem, SessionCommand, SubmitCommandResult } from "./types.ts";

export class DaemonClient {
  private readonly baseUrl: string;

  constructor(baseUrl: string) {
    this.baseUrl = baseUrl;
  }

  async healthCheck(): Promise<boolean> {
    try {
      const res = await fetch(`${this.baseUrl}/health`);
      if (!res.ok) return false;
      const data = (await res.json()) as { status?: string };
      return data.status === "ok" || data.status === "healthy";
    } catch {
      return false;
    }
  }

  async submitCommand(command: SessionCommand): Promise<SubmitCommandResult> {
    try {
      const res = await fetch(`${this.baseUrl}/api/v1/commands`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(command),
      });

      if (res.status === 409) {
        return {
          ok: false,
          error: "Conflict: Command payload differs from previously recorded intent",
        };
      }

      if (!res.ok) {
        const text = await res.text();
        return { ok: false, error: `Daemon error (${res.status}): ${text}` };
      }

      let body: { command_id: string; status: string };
      try {
        body = (await res.json()) as { command_id: string; status: string };
      } catch {
        return { ok: false, error: `Invalid response body (${res.status})` };
      }
      return { ok: true, command_id: body.command_id, status: body.status };
    } catch (err) {
      return { ok: false, error: `Network error: ${String(err)}` };
    }
  }

  async getEvents(runId: string, afterSeq?: number): Promise<DaemonEventItem[]> {
    const query = afterSeq !== undefined ? `?after_seq=${afterSeq}` : "";
    const res = await fetch(
      `${this.baseUrl}/api/v1/runs/${encodeURIComponent(runId)}/events${query}`,
    );
    if (!res.ok) {
      throw new Error(`Failed to fetch events (${res.status}): ${await res.text()}`);
    }
    return (await res.json()) as DaemonEventItem[];
  }
}
