import assert from "node:assert/strict";
import { execFileSync, spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
    existsSync,
    mkdirSync,
    mkdtempSync,
    readFileSync,
    realpathSync,
    rmSync,
    statSync,
    unlinkSync,
} from "node:fs";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const PACKAGE_SCRIPT = join(REPO_ROOT, "scripts", "package-release.mjs");

function sha256File(filePath) {
    return createHash("sha256").update(readFileSync(filePath)).digest("hex");
}

function getFreePort() {
    return new Promise((resolvePort, reject) => {
        const srv = createServer();
        srv.listen(0, "127.0.0.1", () => {
            const addr = srv.address();
            const port = typeof addr === "object" && addr ? addr.port : 0;
            srv.close(() => resolvePort(port));
        });
        srv.on("error", reject);
    });
}

async function waitForHealth(url, timeoutMs = 10_000) {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
        try {
            const res = await fetch(url);
            if (res.ok) {
                return res;
            }
        } catch {
            // Daemon still starting.
        }
        await new Promise((r) => setTimeout(r, 100));
    }
    throw new Error(`Timed out waiting for ${url}`);
}

test("package-release rejects invalid arguments and path traversal", () => {
    const badProfile = spawnSync(process.execPath, [PACKAGE_SCRIPT, "--profile", "invalid"], {
        encoding: "utf8",
    });
    assert.notEqual(badProfile.status, 0);
    assert.match(badProfile.stderr, /Invalid --profile/);

    const badTarget = spawnSync(
        process.execPath,
        [PACKAGE_SCRIPT, "--skip-build", "--target", "../escape"],
        { encoding: "utf8" },
    );
    assert.notEqual(badTarget.status, 0);
    assert.match(badTarget.stderr, /Invalid target triple/);

    const badVersion = spawnSync(
        process.execPath,
        [PACKAGE_SCRIPT, "--skip-build", "--version", "1.0/bad"],
        { encoding: "utf8" },
    );
    assert.notEqual(badVersion.status, 0);
    assert.match(badVersion.stderr, /Invalid version identifier/);
});

test("release bundle verifies checksums, runs standalone daemon + bundled workflow, and configures DSH profile", async () => {
    const workDir = mkdtempSync(join(tmpdir(), "runlark-pkg-test-"));
    const outDir = join(workDir, "dist");
    const extractDir = join(workDir, "extracted");
    const unrelatedCwd = join(workDir, "unrelated-cwd");
    const daemonDataDir = join(workDir, "daemon-data");
    const dshHome = join(workDir, "dsh-home");
    let daemonProc = null;

    try {
        mkdirSync(outDir, { recursive: true });
        mkdirSync(extractDir, { recursive: true });
        mkdirSync(unrelatedCwd, { recursive: true });
        mkdirSync(daemonDataDir, { recursive: true });
        mkdirSync(dshHome, { recursive: true });

        const rawJson = execFileSync(
            process.execPath,
            [PACKAGE_SCRIPT, "--profile", "debug", "--out-dir", outDir, "--json"],
            { cwd: REPO_ROOT, encoding: "utf8" },
        );
        const summary = JSON.parse(rawJson);

        assert.equal(summary.profile, "debug");
        assert.ok(existsSync(summary.archive_path), "archive must exist");
        assert.ok(existsSync(summary.checksum_path), "checksum file must exist");

        const actualArchiveSha = sha256File(summary.archive_path);
        assert.equal(summary.archive_sha256, actualArchiveSha);
        const checksumLine = readFileSync(summary.checksum_path, "utf8").trim();
        assert.equal(checksumLine, `${actualArchiveSha}  ${summary.bundle_name}.tar.gz`);

        // Verify no AppleDouble ._ metadata entries leaked into the tarball.
        const tarEntries = execFileSync("tar", ["-tzf", summary.archive_path], { encoding: "utf8" })
            .split(/\r?\n/)
            .filter(Boolean);
        assert.equal(
            tarEntries.some((entry) => /(?:^|\/)\._/.test(entry)),
            false,
            "tarball must not contain AppleDouble ._* entries",
        );

        execFileSync("tar", ["-xzf", summary.archive_path, "-C", extractDir]);
        const bundleDir = join(extractDir, summary.bundle_name);

        // Verify SHA256SUMS and manifest.json against extracted contents.
        const sumsLines = readFileSync(join(bundleDir, "SHA256SUMS"), "utf8").trim().split("\n");
        assert.ok(sumsLines.length >= 10, "SHA256SUMS must cover all bundle files");
        for (const line of sumsLines) {
            const match = line.match(/^([0-9a-f]{64}) {2}(.+)$/);
            assert.ok(match, `Invalid SHA256SUMS line: ${line}`);
            const [, expectedHash, relPath] = match;
            const fullPath = join(bundleDir, relPath);
            assert.ok(existsSync(fullPath), `Missing file listed in SHA256SUMS: ${relPath}`);
            assert.equal(sha256File(fullPath), expectedHash, `Checksum mismatch for ${relPath}`);
        }

        const manifest = JSON.parse(readFileSync(join(bundleDir, "manifest.json"), "utf8"));
        assert.equal(manifest.license, "Apache-2.0");
        for (const item of manifest.files) {
            const fullPath = join(bundleDir, item.path);
            assert.equal(statSync(fullPath).size, item.bytes, `Byte size mismatch: ${item.path}`);
            assert.equal(sha256File(fullPath), item.sha256, `Manifest sha mismatch: ${item.path}`);
        }

        const licenseText = readFileSync(join(bundleDir, "LICENSE"), "utf8");
        assert.match(licenseText, /Apache License/);
        assert.match(licenseText, /Version 2\.0/);
        assert.ok(existsSync(join(bundleDir, "RUNNING.md")));

        // Verify runtime DSH plugin packaging: no devDependencies, no src/ui, prebuilt client.js exists.
        const pluginPkg = JSON.parse(
            readFileSync(join(bundleDir, "dsh-plugin", "package.json"), "utf8"),
        );
        assert.equal(pluginPkg.license, "Apache-2.0");
        assert.equal(pluginPkg.devDependencies, undefined);
        assert.equal(existsSync(join(bundleDir, "dsh-plugin", "src", "ui")), false);
        assert.ok(existsSync(join(bundleDir, "dsh-plugin", "dist", "client.js")));

        // Verify packaged CLI validates packaged solo-default WASM workflow from outside repo.
        const runlarkBin = join(bundleDir, "bin", "runlark");
        const serverBin = join(bundleDir, "bin", "runlark-server");
        const validateOut = execFileSync(
            runlarkBin,
            ["plugin", "validate", join(bundleDir, "workflows", "solo-default"), "--json"],
            { cwd: unrelatedCwd, encoding: "utf8" },
        );
        assert.equal(JSON.parse(validateOut).validation.verdict, "pass");

        const serverHelp = execFileSync(serverBin, ["--help"], {
            cwd: unrelatedCwd,
            encoding: "utf8",
        });
        assert.match(serverHelp, /serve/);
        assert.match(serverHelp, /bootstrap-owner/);

        // Start packaged daemon from an unrelated empty CWD without RUNLARK_WORKFLOW_PACKAGE
        // and prove it auto-discovers <bundle>/workflows/solo-default.
        const port = await getFreePort();
        const daemonEnv = { ...process.env };
        delete daemonEnv.RUNLARK_WORKFLOW_PACKAGE;
        delete daemonEnv.RUNLARK_DSH_URL;
        delete daemonEnv.RUNLARK_DSH_TOKEN;
        daemonProc = spawn(
            runlarkBin,
            ["daemon", "--host", "127.0.0.1", "--port", String(port), "--data-dir", daemonDataDir],
            { cwd: unrelatedCwd, env: daemonEnv, stdio: ["ignore", "pipe", "pipe"] },
        );

        await waitForHealth(`http://127.0.0.1:${port}/health`);
        const pluginsRes = await fetch(`http://127.0.0.1:${port}/api/v1/plugins`);
        assert.equal(pluginsRes.status, 200);
        const plugins = await pluginsRes.json();
        assert.ok(
            Array.isArray(plugins) && plugins.some((p) => p.name === "solo-default"),
            `Bundled daemon must auto-install solo-default workflow, got: ${JSON.stringify(plugins)}`,
        );

        daemonProc.kill("SIGTERM");
        await new Promise((resolveExit) => daemonProc.once("exit", resolveExit));
        daemonProc = null;

        // Exercise standalone DSH profile setup directly from extracted bundle (no repo sources).
        const setupScript = join(bundleDir, "scripts", "setup-dsh-profile.mjs");
        execFileSync(
            process.execPath,
            [setupScript, "--no-install", "--daemon-url", "http://127.0.0.1:19876"],
            { cwd: unrelatedCwd, env: { ...process.env, DSH_HOME: dshHome }, stdio: "pipe" },
        );

        const profileDir = join(dshHome, "profiles", "runlark");
        const profilePkg = JSON.parse(readFileSync(join(profileDir, "package.json"), "utf8"));
        assert.equal(
            profilePkg.dependencies["@runlark/dsh-plugin"],
            `link:${realpathSync(join(bundleDir, "dsh-plugin"))}`,
        );
        const cordisOverlay = readFileSync(join(profileDir, "cordis.patch.yml"), "utf8");
        assert.match(cordisOverlay, /http:\/\/127\.0\.0\.1:19876/);
        assert.ok(
            existsSync(
                join(
                    profileDir,
                    "patches",
                    "@deepseek-ai+dsh-api-session-controller@0.2.2-rc.3.patch",
                ),
            ),
        );

        // Failure path: invalid --daemon-url and path-traversal --profile rejected before writing profile.
        const badUrl = spawnSync(
            process.execPath,
            [setupScript, "--no-install", "--daemon-url", "javascript:alert(1)"],
            { cwd: unrelatedCwd, env: { ...process.env, DSH_HOME: dshHome }, encoding: "utf8" },
        );
        assert.notEqual(badUrl.status, 0);
        assert.match(badUrl.stderr, /Invalid --daemon-url/);

        const badProfileName = spawnSync(
            process.execPath,
            [setupScript, "--no-install", "--profile", ".."],
            { cwd: unrelatedCwd, env: { ...process.env, DSH_HOME: dshHome }, encoding: "utf8" },
        );
        assert.notEqual(badProfileName.status, 0);
        assert.match(badProfileName.stderr, /Invalid --profile/);

        // Failure path: missing prebuilt client bundle in extracted bundle fails closed.
        unlinkSync(join(bundleDir, "dsh-plugin", "dist", "client.js"));
        const missingClient = spawnSync(process.execPath, [setupScript, "--no-install"], {
            cwd: unrelatedCwd,
            env: { ...process.env, DSH_HOME: dshHome },
            encoding: "utf8",
        });
        assert.notEqual(missingClient.status, 0);
        assert.match(missingClient.stderr, /Prebuilt DSH client bundle missing/);
    } finally {
        if (daemonProc) {
            daemonProc.kill("SIGKILL");
        }
        rmSync(workDir, { recursive: true, force: true });
    }
});
