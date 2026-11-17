import type { Context } from "@deepseek-ai/cordis";
import z from "@deepseek-ai/schemastery";
import { RunlarkHostBridge } from "./bridge.ts";
import { DaemonClient } from "./daemon_client.ts";
import { RunlarkSessionInterceptor } from "./interceptor.ts";

export interface RunlarkPluginConfig {
  readonly daemonUrl: string;
  readonly syncOnStart: boolean;
}

export const name = "runlark";

export const inject = ["sessionController", "agents", "sessions", "workspaceRegistry"];

export const Config: z<Partial<RunlarkPluginConfig>, RunlarkPluginConfig> = z.object({
  daemonUrl: z.string().default("http://127.0.0.1:9876"),
  syncOnStart: z.boolean().default(true),
});

export function apply(ctx: Context, rawConfig?: Partial<RunlarkPluginConfig>): RunlarkHostBridge {
  const config = Config(rawConfig ?? {});
  const client = new DaemonClient(config.daemonUrl);
  const interceptor = new RunlarkSessionInterceptor(client);
  const bridge = new RunlarkHostBridge(ctx, client, interceptor);

  // Provide interceptor synchronously so SessionCommandController picks it up dynamically via ctx.get().
  ctx.provide("sessionCommandInterceptor", interceptor);
  bridge.registerCordisListeners();

  if (config.syncOnStart) {
    void bridge.syncManagedSessionsFromDaemon().catch((error: unknown) => {
      ctx.logger?.warn?.("runlark: initial daemon session sync failed: %s", error);
    });
  }

  return bridge;
}
