#!/usr/bin/env node
import { existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { Context } from "@deepseek-ai/cordis";
import { TypertGatewayService } from "@deepseek-ai/dsh-api-gateway";
import { RemoteAccessService } from "@deepseek-ai/dsh-api-remote-access";
import { SessionController } from "@deepseek-ai/dsh-api-session-controller";
import * as ClientConnectionPlugin from "@deepseek-ai/dsh-client-connection";
import { WebServer } from "@deepseek-ai/dsh-host-webserver";
import { ToolCallId } from "@deepseek-ai/dsh-llm";
import { SessionId, SessionLogOffset, SessionSeq } from "@deepseek-ai/dsh-session";
import { SessionQueryError } from "@deepseek-ai/dsh-session-query";
import { TypertRegistry } from "@deepseek-ai/dsh-typert-registry";
import UserQuestionPlugin from "@deepseek-ai/dsh-user-questions";
import { DaemonClient } from "../packages/dsh-plugin/src/daemon_client.ts";
import { apply as applyRunlarkPlugin } from "../packages/dsh-plugin/src/index.ts";
import { handleRunlarkWebRequest } from "../packages/dsh-plugin/src/web_routes.ts";

function parseArgs(argv) {
    let daemonUrl = "http://127.0.0.1:9876";
    let stateDir = process.cwd();
    let port = 0;

    for (let i = 0; i < argv.length; i += 1) {
        const arg = argv[i];
        if (arg === "--daemon-url" && i + 1 < argv.length) {
            daemonUrl = argv[++i];
        } else if (arg === "--state-dir" && i + 1 < argv.length) {
            stateDir = argv[++i];
        } else if (arg === "--port" && i + 1 < argv.length) {
            port = Number(argv[++i]);
        } else {
            throw new Error(`Unknown argument: ${arg}`);
        }
    }
    return { daemonUrl, stateDir, port };
}

function readJson(path, fallback) {
    if (!existsSync(path)) return fallback;
    try {
        return JSON.parse(readFileSync(path, "utf8"));
    } catch {
        return fallback;
    }
}

function writeJson(path, value) {
    writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`, "utf8");
}

function readBodyText(req) {
    return new Promise((resolve, reject) => {
        const chunks = [];
        req.on("data", (c) => chunks.push(c));
        req.on("end", () => resolve(Buffer.concat(chunks).toString("utf8")));
        req.on("error", reject);
    });
}

function replayBodyStream(rawBody) {
    const buf = Buffer.from(rawBody, "utf8");
    return {
        async *[Symbol.asyncIterator]() {
            if (buf.length > 0) yield buf;
        },
    };
}

const { daemonUrl, stateDir, port } = parseArgs(process.argv.slice(2));
const sessionsDir = join(stateDir, "sessions");
const credPath = join(stateDir, "credentials.json");
const faultsPath = join(stateDir, "faults.json");
const archivePath = join(stateDir, "archived.json");
mkdirSync(sessionsDir, { recursive: true });

const ctx = new Context();
const agentsMap = new Map();
const sessionsMap = new Map();
const persistedSessions = new Map();
const archivedIds = readJson(archivePath, []);
const turnCancels = new Map();

function savePersistedSession(record) {
    persistedSessions.set(record.id, record);
    writeJson(join(sessionsDir, `${record.id}.json`), record);
}

function loadPersistedSessions() {
    for (const file of readdirSync(sessionsDir)) {
        if (!file.endsWith(".json")) continue;
        const rec = readJson(join(sessionsDir, file), null);
        if (rec?.id) {
            persistedSessions.set(rec.id, rec);
        }
    }
}

loadPersistedSessions();

function appendSessionEvent(record, type, data) {
    const seq = SessionSeq(record.events.length);
    const ev = {
        type,
        seq,
        time: Date.now(),
        surfaceOp: "append",
        data,
    };
    record.events.push(ev);
    savePersistedSession(record);
    return ev;
}

function createLiveAgent(record) {
    const sid = SessionId(record.id);
    const session = {
        id: sid,
        seq: SessionLogOffset(record.events.length),
        inheritedEventCount: SessionLogOffset(0),
        header: {
            version: 4,
            id: sid,
            createdAt: record.header.createdAt,
            cwd: record.header.cwd,
            isSeeded: false,
        },
        snapshotEvents: () => [...record.events],
    };
    sessionsMap.set(record.id, session);

    let turnCounter = record.events.filter((e) => e.type === "turn/start").length;

    const emitWire = (type, data) => {
        const ev = appendSessionEvent(record, type, data);
        session.seq = SessionLogOffset(record.events.length);
        ctx.emit("session/event", session, ev);
        return ev;
    };

    const agent = {
        id: sid,
        session,
        status: "idle",
        inbox: { nextTurn: [], nextStep: [] },
        model: { current: () => undefined },
        whenIdle: async () => {},
        followup(msg) {
            this.inbox.nextTurn.push(msg);
            const text = (msg.content ?? [])
                .map((p) => (p.type === "text" ? p.text : ""))
                .join("\n")
                .trim();

            queueMicrotask(() => {
                void runTurn(text, msg.source);
            });
        },
        steer() {},
        cancel(cause, opts) {
            const ac = turnCancels.get(record.id);
            if (ac && !ac.signal.aborted) {
                ac.abort({ cause, opts });
            }
        },
    };

    async function runTurn(promptText, source) {
        this_status("running");
        turnCounter += 1;
        const turn = turnCounter;
        const ac = new AbortController();
        turnCancels.set(record.id, ac);

        emitWire("user/message", {
            source: source ?? { kind: "user" },
            content: [{ type: "text", text: promptText }],
        });
        emitWire("turn/start", { turn });

        try {
            if (promptText.includes("ASK_QUESTION")) {
                const callId = `q_e2e_${turn}`;
                const qPrompt = "Approve deployment from DSH worktree?";
                const qItem = {
                    id: "q_1",
                    question: qPrompt,
                    options: [{ label: "Approve" }, { label: "Reject" }],
                };
                emitWire("tool/call", {
                    turn,
                    step: 1,
                    callId,
                    name: "ask_user_question",
                    arguments: JSON.stringify({ questions: [qItem] }),
                });

                const answer = await ctx.waterfall(
                    "user-questions/request",
                    {
                        agent,
                        questions: [qItem],
                        wait: { callId: ToolCallId(callId), timed: true },
                        signal: ac.signal,
                    },
                    async () => {
                        const err = new Error("no browser client attached");
                        err.code = "NO_PROVIDER";
                        throw err;
                    },
                );
                emitWire("tool/result", {
                    turn,
                    step: 1,
                    callId,
                    name: "ask_user_question",
                    content: [{ type: "text", text: JSON.stringify(answer) }],
                });
                emitWire("turn/end", { turn, reason: { kind: "completed" } });
            } else if (promptText.includes("WAIT_CANCEL")) {
                await new Promise((resolve) => {
                    if (ac.signal.aborted) {
                        resolve();
                        return;
                    }
                    const timer = setTimeout(resolve, 15_000);
                    ac.signal.addEventListener(
                        "abort",
                        () => {
                            clearTimeout(timer);
                            setTimeout(resolve, 120);
                        },
                        { once: true },
                    );
                });
                if (ac.signal.aborted) {
                    emitWire("turn/end", {
                        turn,
                        reason: { kind: "aborted", reason: { kind: "user" } },
                    });
                } else {
                    emitWire("turn/end", { turn, reason: { kind: "completed" } });
                }
            } else {
                await new Promise((r) => setTimeout(r, 40));
                if (ac.signal.aborted) {
                    emitWire("turn/end", {
                        turn,
                        reason: { kind: "aborted", reason: { kind: "user" } },
                    });
                } else {
                    emitWire("assistant/message", {
                        turn,
                        step: 1,
                        content: [{ type: "text", text: `Processed: ${promptText}` }],
                    });
                    emitWire("turn/end", { turn, reason: { kind: "completed" } });
                }
            }
        } finally {
            if (turnCancels.get(record.id) === ac) {
                turnCancels.delete(record.id);
            }
            this_status("idle");
        }
    }

    function this_status(status) {
        agent.status = status;
    }

    agentsMap.set(record.id, agent);
    return agent;
}

ctx.provide("credentials", {
    async modifyRecord(key, updater) {
        const store = readJson(credPath, {});
        const next = await updater(store[key]);
        if (next !== undefined) {
            store[key] = next;
            writeJson(credPath, store);
        }
        return store[key];
    },
});

ctx.provide("agentDefaultModel", {
    currentSelection: () => ({ provider: "mock", model: "mock-echo" }),
});
ctx.provide("agents", {
    get: (id) => agentsMap.get(id),
    list: () => [...agentsMap.values()],
    create: async (opts) => {
        const sid = opts.sessionId ?? `session-${Date.now()}`;
        const record = {
            id: sid,
            header: {
                version: 4,
                id: sid,
                createdAt: Date.now(),
                cwd: opts.meta?.cwd ?? opts.cwd ?? stateDir,
                isSeeded: false,
            },
            events: [],
        };
        savePersistedSession(record);
        return { agent: createLiveAgent(record) };
    },
    resume: async ({ resumeSessionId }) => {
        const existing = agentsMap.get(resumeSessionId);
        if (existing) return { agent: existing };
        const rec = persistedSessions.get(resumeSessionId);
        if (!rec) {
            throw new SessionQueryError(
                `not found: ${resumeSessionId}`,
                "SESSION_QUERY_SESSION_NOT_FOUND",
            );
        }
        return { agent: createLiveAgent(rec) };
    },
});
ctx.provide("attachments", {
    imageLimits: { maxMessageImageBytes: 1024 * 1024 },
    admitPromptContent: async (c) => c,
});
ctx.provide("fileUploads", {
    registerAgentResolver: () => () => {},
    bindPrompt: () => ({ commit() {}, [Symbol.dispose]() {} }),
});
ctx.provide("fs", {});
ctx.provide("llm", {
    listProviders: () => [],
    listModels: async () => [],
    listConfigurableProviders: () => [],
});
ctx.provide("sessions", {
    get: (id) => sessionsMap.get(id),
    list: () => [...sessionsMap.values()],
});
ctx.provide("sessionProjections", {
    register: async () => ({ stateOf: () => ({ ending: null }), [Symbol.dispose]() {} }),
    onEventChanged: () => () => {},
});
ctx.provide("sessionQuery", {
    listHeaders: async () => [...persistedSessions.values()].map((r) => r.header),
    observeSession: async (id) => {
        const rec = persistedSessions.get(id);
        if (!rec) {
            throw new SessionQueryError(`not found: ${id}`, "SESSION_QUERY_SESSION_NOT_FOUND");
        }
        const cursor = rec.events.length - 1;
        return {
            header: rec.header,
            inheritedEventCount: SessionLogOffset(0),
            events: rec.events,
            cursor,
            projections: { asOfSeq: cursor, values: {} },
            source: "prepared",
            retain() {
                return this;
            },
            [Symbol.dispose]() {},
        };
    },
});
ctx.provide("workspaceRegistry", {
    list: () => [],
    archiveSession: async (sid) => {
        if (!archivedIds.includes(sid)) {
            archivedIds.push(sid);
            writeJson(archivePath, archivedIds);
        }
    },
});

await ctx.plugin(WebServer, { host: "127.0.0.1", port });
await ctx.plugin(ClientConnectionPlugin);
await ctx.plugin(TypertRegistry);
await ctx.plugin(RemoteAccessService);
await ctx.plugin(TypertGatewayService);
await ctx.plugin(UserQuestionPlugin);
await ctx.plugin(SessionController, { nativeOpen: false });

const bridge = applyRunlarkPlugin(ctx, { daemonUrl, syncOnStart: true });
const daemonClient = new DaemonClient(daemonUrl);

// Exact-route fault injector for /api/runlark/dispatch; delegates to handleRunlarkWebRequest.
ctx.webServer.register({
    kind: "exact",
    path: "/api/runlark/dispatch",
    handler: async (req, res) => {
        const rawBody = await readBodyText(req);
        let parsed = {};
        try {
            parsed = rawBody ? JSON.parse(rawBody) : {};
        } catch {
            // Let handleRunlarkWebRequest reject malformed JSON
        }
        const faults = readJson(faultsPath, { failOnce: {}, unknownOnce: {} });
        const cmdId = String(parsed.command_id ?? "");
        if (cmdId && faults.failOnce?.[cmdId]) {
            delete faults.failOnce[cmdId];
            writeJson(faultsPath, faults);
            res.writeHead(500, { "Content-Type": "application/json" });
            res.end(JSON.stringify({ error: "simulated_dsh_dispatch_failure" }));
            return;
        }
        if (cmdId && faults.unknownOnce?.[cmdId]) {
            delete faults.unknownOnce[cmdId];
            writeJson(faultsPath, faults);
            res.writeHead(504, { "Content-Type": "application/json" });
            res.end(JSON.stringify({ error: "simulated_gateway_timeout" }));
            return;
        }
        const syntheticReq = Object.assign(replayBodyStream(rawBody), {
            method: req.method,
            url: req.url,
            headers: req.headers,
        });
        await handleRunlarkWebRequest(syntheticReq, res, bridge, daemonClient);
    },
});

const origin = ctx.webServer.getOrigin().replace(/\/+$/u, "");
const authUrl = ctx.connection.browserAuth.authenticatedUrl(`${origin}/`);
const token = new URL(authUrl).searchParams.get("token");

console.log(`READY ${JSON.stringify({ url: authUrl, origin, token })}`);

const shutdown = async () => {
    await ctx.fiber.dispose();
    process.exit(0);
};
process.on("SIGTERM", () => void shutdown());
process.on("SIGINT", () => void shutdown());
