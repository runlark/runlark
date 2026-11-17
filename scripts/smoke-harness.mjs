import assert from "node:assert/strict";
import { execFileSync, spawn } from "node:child_process";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import net from "node:net";
import { join } from "node:path";
import { createRequire } from "node:module";
import { DatabaseSync } from "node:sqlite";
import { Context as CordisContext } from "@deepseek-ai/cordis";
import { JSDOM } from "jsdom";
import React, { act } from "react";
import ReactDom from "react-dom";
import * as ReactDomClient from "react-dom/client";
import { createRoot } from "react-dom/client";
import * as ReactJsxRuntime from "react/jsx-runtime";

const require = createRequire(import.meta.url);

export const getFreePort = () =>
    new Promise((resolve, reject) => {
        const srv = net.createServer();
        srv.listen(0, "127.0.0.1", () => {
            const port = srv.address().port;
            srv.close((err) => (err ? reject(err) : resolve(port)));
        });
    });

export async function waitFor(check, timeoutMs = 8000, label = "condition") {
    const deadline = Date.now() + timeoutMs;
    let lastError;
    while (Date.now() < deadline) {
        try {
            const result = await check();
            if (result !== false) {
                return result;
            }
        } catch (err) {
            lastError = err;
        }
        await new Promise((r) => setTimeout(r, 40));
    }
    throw lastError ?? new Error(`Timed out waiting for ${label} after ${timeoutMs}ms`);
}

export function initGitRepo(repoDir) {
    const git = (...args) =>
        execFileSync("git", ["-C", repoDir, ...args], { encoding: "utf8", stdio: "pipe" }).trim();
    git("init");
    git("config", "user.email", "smoke@runlark.dev");
    git("config", "user.name", "Runlark Smoke");
    writeFileSync(join(repoDir, "README.md"), "# Runlark E2E Smoke\n", "utf8");
    git("add", "README.md");
    git("commit", "-m", "init");
    return git;
}

export async function startDaemon(binaryPath, dataDir, port, dshUrl) {
    const daemonUrl = `http://127.0.0.1:${port}`;
    const args = ["daemon", "--data-dir", dataDir, "--port", String(port)];
    if (dshUrl) {
        args.push("--dsh-url", dshUrl);
    }
    const proc = spawn(binaryPath, args, { stdio: ["ignore", "pipe", "pipe"] });
    let stdout = "";
    let stderr = "";
    proc.stdout.on("data", (d) => {
        stdout += d.toString();
    });
    proc.stderr.on("data", (d) => {
        stderr += d.toString();
    });

    await waitFor(
        async () => {
            const res = await fetch(`${daemonUrl}/health`, { signal: AbortSignal.timeout(1000) });
            if (!res.ok) return false;
            const data = await res.json();
            return data.status === "ok" || data.status === "healthy";
        },
        10_000,
        "daemon health",
    ).catch((err) => {
        proc.kill("SIGKILL");
        throw new Error(
            `Daemon failed to start: ${err.message}\nSTDOUT:${stdout}\nSTDERR:${stderr}`,
        );
    });

    return { proc, daemonUrl, getLogs: () => ({ stdout, stderr }) };
}

export async function startDshHost(daemonUrl, stateDir, port) {
    const proc = spawn(
        process.execPath,
        [
            join(process.cwd(), "scripts", "smoke-dsh-host.mjs"),
            "--daemon-url",
            daemonUrl,
            "--state-dir",
            stateDir,
            "--port",
            String(port),
        ],
        { stdio: ["ignore", "pipe", "pipe"] },
    );
    let stdout = "";
    let stderr = "";
    proc.stdout.on("data", (d) => {
        stdout += d.toString();
    });
    proc.stderr.on("data", (d) => {
        stderr += d.toString();
    });

    const ready = await waitFor(
        () => {
            const match = /^READY (\{.*\})$/m.exec(stdout);
            return match ? JSON.parse(match[1]) : false;
        },
        10_000,
        "DSH host READY",
    ).catch((err) => {
        proc.kill("SIGKILL");
        throw new Error(`DSH host failed to start: ${err.message}\nOUT:${stdout}\nERR:${stderr}`);
    });

    return { proc, ...ready, getLogs: () => ({ stdout, stderr }) };
}

export async function stopProcess(handle, label) {
    if (!handle || handle.proc.exitCode !== null) return;
    const result = await new Promise((resolve, reject) => {
        const timer = setTimeout(() => {
            handle.proc.kill("SIGKILL");
            const logs = handle.getLogs();
            reject(
                new Error(
                    `${label} did not exit cleanly within 5s\nOUT:${logs.stdout}\nERR:${logs.stderr}`,
                ),
            );
        }, 5000);
        handle.proc.once("exit", (code, signal) => {
            clearTimeout(timer);
            resolve({ code, signal });
        });
        handle.proc.kill("SIGTERM");
    });
    assert.equal(result.code, 0, `${label} exited with code ${result.code}`);
}

export async function crashProcess(handle, label) {
    if (!handle || handle.proc.exitCode !== null) return;
    await new Promise((resolve, reject) => {
        const timer = setTimeout(() => {
            reject(new Error(`${label} did not exit after SIGKILL within 5s`));
        }, 5000);
        handle.proc.once("exit", () => {
            clearTimeout(timer);
            resolve();
        });
        handle.proc.kill("SIGKILL");
    });
}

export async function apiJson(url, init) {
    const res = await fetch(url, init);
    const text = await res.text();
    const json = text ? JSON.parse(text) : null;
    return { status: res.status, ok: res.ok, json, text };
}

export function queryDb(dbPath, sql, ...params) {
    const db = new DatabaseSync(dbPath, { readOnly: true });
    try {
        return db.prepare(sql).all(...params);
    } finally {
        db.close();
    }
}

export function writeMockLlmPatch(dshHome) {
    const dshLlmUrl = new URL("../node_modules/@deepseek-ai/dsh-llm/lib/index.js", import.meta.url)
        .href;
    const mockPluginPath = join(dshHome, "mock-llm.mjs");
    writeFileSync(
        mockPluginPath,
        `import { LlmAdapter, ToolCallId } from ${JSON.stringify(dshLlmUrl)};
class MockLlmAdapter extends LlmAdapter {
    providerInfo(provider) { return { id: provider, name: "Runlark Mock LLM" }; }
    async listModels() { return [{ id: "mock-echo", name: "Mock Echo" }]; }
    async *stream(options) {
        if (options.purpose) {
            yield { type: "block-start", index: 0, blockType: "text" };
            yield { type: "text-delta", index: 0, text: "Runlark Mock Title" };
            yield { type: "block-end", index: 0, block: { type: "text", text: "Runlark Mock Title" } };
            yield { type: "usage", usage: { inputTokens: 2, outputTokens: 2 } };
            yield { type: "finish", reason: { kind: "stop" } };
            return;
        }
        const last = options.messages.at(-1);
        const isRealUserPrompt = (m) =>
            m.role === "user" && !m.source?.form && m.source?.kind !== "user-question-reply";
        const lastPromptIdx = options.messages.findLastIndex(isRealUserPrompt);
        const turnMessages = lastPromptIdx >= 0 ? options.messages.slice(lastPromptIdx) : options.messages;
        const lastPromptText = lastPromptIdx >= 0 ? JSON.stringify(options.messages[lastPromptIdx]) : "";
        const hasAskToolResult = turnMessages.some(
            (m) =>
                (m.role === "tool" && JSON.stringify(m).includes("q_real_dsh_1")) ||
                (m.role === "user" && m.source?.kind === "user-question-reply") ||
                (m.role === "user" && JSON.stringify(m).includes("Answer to question")),
        );
        if (hasAskToolResult || last?.role === "tool") {
            yield { type: "block-start", index: 0, blockType: "text" };
            yield { type: "text-delta", index: 0, text: "Approved and completed." };
            yield { type: "block-end", index: 0, block: { type: "text", text: "Approved and completed." } };
            yield { type: "usage", usage: { inputTokens: 4, outputTokens: 4 } };
            yield { type: "finish", reason: { kind: "stop" } };
            return;
        }
        if (lastPromptText.includes("ASK_QUESTION")) {
            const callId = ToolCallId("q_real_dsh_1");
            const args = JSON.stringify({
                questions: [{ id: "q1", question: "Approve real DSH tool execution?", options: [{ label: "Approve" }, { label: "Reject" }] }],
            });
            yield { type: "block-start", index: 0, blockType: "tool-call" };
            yield { type: "tool-call-delta", index: 0, id: callId, name: "ask_user_question", argumentsDelta: args };
            yield { type: "block-end", index: 0, block: { type: "tool-call", id: callId, name: "ask_user_question", arguments: args } };
            yield { type: "usage", usage: { inputTokens: 4, outputTokens: 8 } };
            yield { type: "finish", reason: { kind: "tool-calls" } };
            return;
        }
        if (lastPromptText.includes("WAIT_CANCEL")) {
            await new Promise((resolve) => {
                if (options.signal?.aborted) { resolve(); return; }
                const t = setTimeout(resolve, 15_000);
                options.signal?.addEventListener("abort", () => { clearTimeout(t); setTimeout(resolve, 150); }, { once: true });
            });
            if (options.signal?.aborted) {
                yield { type: "finish", reason: { kind: "aborted", failure: { message: "user aborted", code: "ABORTED" } } };
                return;
            }
        }
        yield { type: "block-start", index: 0, blockType: "text" };
        yield { type: "text-delta", index: 0, text: "Mock reply ok." };
        yield { type: "block-end", index: 0, block: { type: "text", text: "Mock reply ok." } };
        yield { type: "usage", usage: { inputTokens: 4, outputTokens: 4 } };
        yield { type: "finish", reason: { kind: "stop" } };
    }
}
export const name = "runlark-smoke-mock-llm";
export const inject = ["llm"];
export function apply(ctx) { ctx.llm.registerAdapter(["runlark-mock"], new MockLlmAdapter()); }
`,
        "utf8",
    );
    const patchYml = join(dshHome, "mock-llm-patch.yml");
    writeFileSync(
        patchYml,
        `- id: agent-default-model\n  config:\n    provider: runlark-mock\n    model: mock-echo\n- insert:\n    - id: runlark-smoke-mock-llm\n      name: ${JSON.stringify(mockPluginPath)}\n`,
        "utf8",
    );
    return patchYml;
}

export function submitCommand(daemonUrl, type, payload) {
    return apiJson(`${daemonUrl}/api/v1/commands`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ type, payload }),
    });
}

export function setDshFault(stateDir, kind, commandId) {
    const path = join(stateDir, "faults.json");
    const current = existsSync(path)
        ? JSON.parse(readFileSync(path, "utf8"))
        : { failOnce: {}, unknownOnce: {} };
    current[kind][commandId] = true;
    writeFileSync(path, `${JSON.stringify(current, null, 2)}\n`, "utf8");
}

export function mountUiCard(dshOrigin, sessionId, cwd, autoOpenExpectation = false) {
    const dom = new JSDOM(
        `<!DOCTYPE html><html lang="en"><body><div id="root"></div></body></html>`,
        {
            url: `${dshOrigin}/`,
            runScripts: "dangerously",
        },
    );
    const prev = {
        window: globalThis.window,
        document: globalThis.document,
        HTMLElement: globalThis.HTMLElement,
        Element: globalThis.Element,
        Node: globalThis.Node,
        getComputedStyle: globalThis.getComputedStyle,
        fetch: globalThis.fetch,
        IS_REACT_ACT_ENVIRONMENT: globalThis.IS_REACT_ACT_ENVIRONMENT,
    };
    Object.assign(globalThis, {
        window: dom.window,
        document: dom.window.document,
        HTMLElement: dom.window.HTMLElement,
        Element: dom.window.Element,
        Node: dom.window.Node,
        getComputedStyle: dom.window.getComputedStyle.bind(dom.window),
        IS_REACT_ACT_ENVIRONMENT: true,
        fetch: (input, init) => {
            const resolved =
                typeof input === "string" && input.startsWith("/") ? `${dshOrigin}${input}` : input;
            return prev.fetch(resolved, init);
        },
    });
    dom.window.fetch = globalThis.fetch;
    dom.window.AbortController = globalThis.AbortController;

    const loaded = new Map();
    const staticModules = {
        react: () => React,
        "react/jsx-runtime": () => ReactJsxRuntime,
        "react-dom": () => ReactDom,
        "react-dom/client": () => ReactDomClient,
        "@deepseek-ai/cordis": () => require("@deepseek-ai/cordis"),
        "@deepseek-ai/dsh-client-store": () => require("@deepseek-ai/dsh-client-store"),
        "@deepseek-ai/dsh-client-ui-slots": () => require("@deepseek-ai/dsh-client-ui-slots"),
        "@deepseek-ai/dsh-client-ui-primitives": () =>
            require("@deepseek-ai/dsh-client-ui-primitives"),
    };
    dom.window.__ModuleLoader__ = {
        load(entry) {
            loaded.set(
                entry.id,
                entry.factory((s) => staticModules[s]()),
            );
        },
    };
    dom.window.eval(
        readFileSync(join(process.cwd(), "packages", "dsh-plugin", "dist", "client.js"), "utf8"),
    );
    const uiPlugin = loaded.get("@runlark/dsh-plugin");
    const rootEl = dom.window.document.getElementById("root");
    const root = createRoot(rootEl);

    return {
        dom,
        rootEl,
        uiPlugin,
        async open() {
            const cordis = new CordisContext();
            const openedTabs = [];
            if (autoOpenExpectation) {
                cordis.provide("sidebarRightTabs", { register: () => () => {} });
                cordis.provide("sidebarRight", {
                    openTab(kind) {
                        openedTabs.push(kind);
                    },
                });
                cordis.provide("slots", {
                    register: () => () => {},
                    inject: (_n, fn) => fn(),
                });
                cordis.provide("uiSession", {
                    adapter: {
                        current: {
                            getSnapshot: () => ({ key: sessionId }),
                            subscribe: () => () => {},
                        },
                    },
                });
                await cordis.plugin(uiPlugin);
            }

            await act(async () => {
                root.render(
                    React.createElement(uiPlugin.RunlarkTabBody, {
                        sessionId,
                        useSessions: (selector) => selector({ byId: { [sessionId]: { cwd } } }),
                        useTabInfo: () => ({ tab: { title: "Runlark" } }),
                    }),
                );
            });
            return {
                openedTabs,
                async close() {
                    await act(async () => {
                        root.unmount();
                    });
                    if (autoOpenExpectation) {
                        await cordis.fiber.dispose();
                    }
                    dom.window.close();
                    Object.assign(globalThis, prev);
                },
            };
        },
    };
}
