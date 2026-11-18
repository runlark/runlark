#!/usr/bin/env node
import { execFileSync, spawnSync } from "node:child_process";
import net from "node:net";

const POSTGRES_IMAGE =
    "postgres:16-alpine@sha256:81e77a4833487b8132fe4b00bedd39dc3f74b8641e387f19312a48f7de218c2e";

function getFreePort() {
    return new Promise((resolve, reject) => {
        const server = net.createServer();
        server.unref();
        server.on("error", reject);
        server.listen(0, "127.0.0.1", () => {
            const addr = server.address();
            const port = typeof addr === "object" && addr ? addr.port : 0;
            server.close((err) => (err ? reject(err) : resolve(port)));
        });
    });
}

function runCommand(cmd, args, env) {
    const res = spawnSync(cmd, args, {
        stdio: "inherit",
        env: { ...process.env, ...env },
    });
    return res.status ?? 1;
}

const args = process.argv.slice(2);
if (args.length === 0) {
    console.error("Usage: node scripts/with-test-postgres.mjs <command> [args...]");
    process.exit(2);
}

const [cmd, ...cmdArgs] = args;

if (process.env.RUNLARK_TEST_POSTGRES_URL) {
    process.exit(runCommand(cmd, cmdArgs, {}));
}

try {
    execFileSync("docker", ["info"], { stdio: "ignore" });
} catch {
    console.error(
        "RUNLARK_TEST_POSTGRES_URL is not set and Docker daemon is unavailable. Real PostgreSQL tests cannot be skipped.",
    );
    process.exit(1);
}

function canConnectTcp(host, port) {
    return new Promise((resolve) => {
        const socket = net.createConnection({ host, port });
        socket.setTimeout(1000);
        socket.once("connect", () => {
            socket.destroy();
            resolve(true);
        });
        socket.once("timeout", () => {
            socket.destroy();
            resolve(false);
        });
        socket.once("error", () => {
            socket.destroy();
            resolve(false);
        });
    });
}

async function waitForPostgresReady(containerName, port) {
    const deadline = Date.now() + 30_000;
    while (Date.now() < deadline) {
        // Check TCP loopback inside container (not Unix socket during initdb) and verify SQL + host port.
        const readyProbe = spawnSync(
            "docker",
            [
                "exec",
                containerName,
                "pg_isready",
                "-h",
                "127.0.0.1",
                "-p",
                "5432",
                "-U",
                "postgres",
                "-d",
                "runlark_test",
            ],
            { stdio: "ignore" },
        );
        if (readyProbe.status === 0) {
            const sqlProbe = spawnSync(
                "docker",
                [
                    "exec",
                    containerName,
                    "psql",
                    "-h",
                    "127.0.0.1",
                    "-p",
                    "5432",
                    "-U",
                    "postgres",
                    "-d",
                    "runlark_test",
                    "-tAc",
                    "SELECT 1",
                ],
                { stdio: "ignore" },
            );
            if (sqlProbe.status === 0 && (await canConnectTcp("127.0.0.1", port))) {
                return;
            }
        }
        await new Promise((r) => setTimeout(r, 250));
    }
    throw new Error("Timed out waiting for ephemeral PostgreSQL container to become ready");
}

const port = await getFreePort();
const containerName = `runlark-pg-test-${process.pid}-${Date.now()}`;
const postgresUrl = `postgres://postgres:postgres@127.0.0.1:${port}/runlark_test`;

try {
    execFileSync(
        "docker",
        [
            "run",
            "-d",
            "--rm",
            "--name",
            containerName,
            "-e",
            "POSTGRES_USER=postgres",
            "-e",
            "POSTGRES_PASSWORD=postgres",
            "-e",
            "POSTGRES_DB=runlark_test",
            "-p",
            `127.0.0.1:${port}:5432`,
            POSTGRES_IMAGE,
        ],
        { stdio: "inherit" },
    );

    await waitForPostgresReady(containerName, port);

    const exitCode = runCommand(cmd, cmdArgs, { RUNLARK_TEST_POSTGRES_URL: postgresUrl });
    process.exitCode = exitCode;
} finally {
    spawnSync("docker", ["rm", "-f", containerName], { stdio: "ignore" });
}
