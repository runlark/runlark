#!/usr/bin/env node
import assert from "node:assert/strict";
import { execFileSync, spawn } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import net from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";

const getFreePort = () =>
    new Promise((resolve, reject) => {
        const srv = net.createServer();
        srv.listen(0, "127.0.0.1", () => {
            const port = srv.address().port;
            srv.close((err) => (err ? reject(err) : resolve(port)));
        });
    });

console.log("Compiling runlark binary...");
execFileSync("cargo", ["build", "--bin", "runlark"], { stdio: "inherit" });

const port = await getFreePort();
const tempDir = mkdtempSync(join(tmpdir(), "runlark-smoke-"));
const binaryPath = join(process.cwd(), "target", "debug", "runlark");

console.log(`Starting runlark daemon on port ${port} with data-dir ${tempDir}...`);
const proc = spawn(binaryPath, ["daemon", "--data-dir", tempDir, "--port", String(port)], {
    stdio: ["ignore", "pipe", "pipe"],
});

let stdout = "";
let stderr = "";
proc.stdout.on("data", (d) => {
    stdout += d.toString();
});
proc.stderr.on("data", (d) => {
    stderr += d.toString();
});

const cleanup = () => {
    try {
        proc.kill("SIGKILL");
    } catch {
        // already stopped
    }
    rmSync(tempDir, { recursive: true, force: true });
};

try {
    const healthUrl = `http://127.0.0.1:${port}/health`;
    let healthy = false;
    const deadline = Date.now() + 10_000;

    while (Date.now() < deadline) {
        try {
            const res = await fetch(healthUrl);
            if (res.ok) {
                const data = await res.json();
                if (data.status === "ok" || data.status === "healthy") {
                    healthy = true;
                    break;
                }
            }
        } catch {
            // Wait for socket to accept connections
        }
        await new Promise((r) => setTimeout(r, 100));
    }

    if (!healthy) {
        throw new Error(
            `Daemon failed health check within 10s.\nSTDOUT: ${stdout}\nSTDERR: ${stderr}`,
        );
    }
    console.log("Daemon responded healthy.");

    // Verify sessions endpoint
    const sessionsUrl = `http://127.0.0.1:${port}/api/v1/sessions`;
    const res = await fetch(sessionsUrl);
    assert.equal(res.status, 200);
    const sessions = await res.json();
    assert.ok(Array.isArray(sessions), "Sessions endpoint must return an array");

    // Verify session creation
    const createRes = await fetch(sessionsUrl, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
            session_id: "smoke_sess_1",
            task_id: "smoke_task_1",
            cwd: tempDir,
        }),
    });
    assert.equal(createRes.status, 201);
    const createdSession = await createRes.json();
    assert.equal(createdSession.session_id, "smoke_sess_1");

    // Verify command submission
    const commandsUrl = `http://127.0.0.1:${port}/api/v1/commands`;
    const cmdRes = await fetch(commandsUrl, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
            type: "SendPrompt",
            payload: {
                command_id: "smoke_cmd_1",
                session_id: "smoke_sess_1",
                run_id: "smoke_run_1",
                prompt: "Smoke test prompt",
                expected_revision: 1,
                created_at: Date.now(),
            },
        }),
    });
    assert.equal(cmdRes.status, 201);
    const cmdBody = await cmdRes.json();
    assert.equal(cmdBody.command_id, "smoke_cmd_1");

    // Graceful shutdown test via SIGTERM
    console.log("Sending SIGTERM to test graceful shutdown...");
    const exitCode = await new Promise((resolve, reject) => {
        const timer = setTimeout(() => {
            proc.kill("SIGKILL");
            reject(new Error("Daemon did not terminate gracefully within 5s"));
        }, 5000);

        proc.on("exit", (code, signal) => {
            clearTimeout(timer);
            resolve({ code, signal });
        });

        proc.kill("SIGTERM");
    });

    assert.equal(exitCode.code, 0, `Daemon exited with non-zero code ${exitCode.code}`);
    console.log(
        "Process smoke test passed: daemon started, responded healthy, and shut down cleanly.",
    );
} finally {
    cleanup();
}
