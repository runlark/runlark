#!/usr/bin/env node
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
    chmodSync,
    copyFileSync,
    existsSync,
    mkdirSync,
    mkdtempSync,
    readdirSync,
    readFileSync,
    rmSync,
    statSync,
    writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const DSH_VERSION = "0.2.2-rc.3";
const PATCH_FILE = `@deepseek-ai+dsh-api-session-controller@${DSH_VERSION}.patch`;
const WORKFLOW_FILES = ["manifest.json", "workflow.json", "validator.wasm", "validator.wat"];

function printHelp() {
    console.log(
        [
            "Usage: node scripts/package-release.mjs [options]",
            "",
            "Packages a standalone Runlark release archive (.tar.gz + .sha256) containing",
            "the runlark CLI/daemon, runlark-server, first-party solo-default workflow, and",
            "prebuilt @runlark/dsh-plugin + profile installer.",
            "",
            "Options:",
            "  --profile <release|debug>  Cargo build profile (default: release)",
            "  --skip-build               Reuse already-built binaries and DSH client bundle",
            "  --out-dir <dir>            Output directory for archive (default: dist)",
            "  --version <ver>            Override bundle version (default: workspace version)",
            "  --target <triple>          Override target triple (default: rustc host triple)",
            "  --json                     Print structured JSON result",
            "  -h, --help                 Show this help message",
        ].join("\n"),
    );
}

function parseArgs(argv) {
    let profile = "release";
    let skipBuild = false;
    let outDir = join(REPO_ROOT, "dist");
    let version = null;
    let target = null;
    let jsonMode = false;

    for (let i = 0; i < argv.length; i += 1) {
        const arg = argv[i];
        if (arg === "--help" || arg === "-h") {
            printHelp();
            process.exit(0);
        } else if (arg === "--profile" && i + 1 < argv.length) {
            profile = argv[i + 1];
            i += 1;
        } else if (arg === "--skip-build") {
            skipBuild = true;
        } else if (arg === "--out-dir" && i + 1 < argv.length) {
            outDir = resolve(argv[i + 1]);
            i += 1;
        } else if (arg === "--version" && i + 1 < argv.length) {
            version = argv[i + 1];
            i += 1;
        } else if (arg === "--target" && i + 1 < argv.length) {
            target = argv[i + 1];
            i += 1;
        } else if (arg === "--json") {
            jsonMode = true;
        } else {
            throw new Error(`Unknown argument: ${arg}`);
        }
    }

    if (profile !== "release" && profile !== "debug") {
        throw new Error(`Invalid --profile '${profile}' (expected 'release' or 'debug')`);
    }

    return { profile, skipBuild, outDir, version, target, jsonMode };
}

function parseCargoPackageVersion(tomlPath) {
    const raw = readFileSync(tomlPath, "utf8");
    const pkgSection = raw.match(/\[package\][\s\S]*?(?=\n\[|$)/);
    const match = pkgSection?.[0].match(/^\s*version\s*=\s*"([^"]+)"/m);
    if (!match) {
        throw new Error(`Could not read [package].version from ${tomlPath}`);
    }
    return match[1];
}

function resolveVersion(override) {
    const cliVersion = parseCargoPackageVersion(
        join(REPO_ROOT, "crates", "runlark-cli", "Cargo.toml"),
    );
    const serverVersion = parseCargoPackageVersion(
        join(REPO_ROOT, "crates", "runlark-server", "Cargo.toml"),
    );
    const pluginPkg = JSON.parse(
        readFileSync(join(REPO_ROOT, "packages", "dsh-plugin", "package.json"), "utf8"),
    );

    if (cliVersion !== serverVersion || cliVersion !== pluginPkg.version) {
        throw new Error(
            `Version mismatch across workspace (cli=${cliVersion}, server=${serverVersion}, dsh-plugin=${pluginPkg.version})`,
        );
    }

    const selected = override ?? cliVersion;
    if (!/^[0-9A-Za-z][0-9A-Za-z._+-]*$/.test(selected) || selected.includes("..")) {
        throw new Error(`Invalid version identifier: ${selected}`);
    }
    return selected;
}

function resolveTargetTriple(override) {
    let selected = override;
    if (!selected) {
        try {
            const rustcInfo = execFileSync("rustc", ["-vV"], { encoding: "utf8" });
            const match = rustcInfo.match(/^host:\s*(\S+)/m);
            selected = match?.[1] ?? `${process.arch}-${process.platform}`;
        } catch {
            selected = `${process.arch}-${process.platform}`;
        }
    }
    if (!/^[0-9A-Za-z][0-9A-Za-z._-]*$/.test(selected) || selected.includes("..")) {
        throw new Error(`Invalid target triple: ${selected}`);
    }
    return selected;
}

function readGitSha() {
    try {
        return execFileSync("git", ["rev-parse", "HEAD"], {
            cwd: REPO_ROOT,
            encoding: "utf8",
        }).trim();
    } catch {
        return "unknown";
    }
}

function sha256File(filePath) {
    return createHash("sha256").update(readFileSync(filePath)).digest("hex");
}

function assertExists(filePath, label) {
    if (!existsSync(filePath)) {
        throw new Error(`Required ${label} missing at ${filePath}`);
    }
}

function copyChecked(src, dest, mode = null) {
    mkdirSync(dirname(dest), { recursive: true });
    copyFileSync(src, dest);
    if (mode !== null) {
        chmodSync(dest, mode);
    }
}

function listFilesRecursive(dir, baseDir = dir) {
    const out = [];
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
        const full = join(dir, entry.name);
        if (entry.isDirectory()) {
            out.push(...listFilesRecursive(full, baseDir));
        } else if (entry.isFile()) {
            out.push(relative(baseDir, full).split("\\").join("/"));
        }
    }
    return out.sort((a, b) => a.localeCompare(b));
}

function buildRuntimePluginPackageJson(sourcePkg) {
    return {
        name: sourcePkg.name,
        version: sourcePkg.version,
        license: sourcePkg.license ?? "Apache-2.0",
        type: sourcePkg.type ?? "module",
        main: sourcePkg.main ?? "./src/index.ts",
        types: sourcePkg.types ?? "./src/index.ts",
        exports: {
            ".": {
                types: "./src/index.ts",
                default: "./src/index.ts",
            },
            "./client": {
                default: "./dist/client.js",
            },
        },
        dsh: sourcePkg.dsh,
        peerDependencies: sourcePkg.peerDependencies,
    };
}

function stageBundle(stageRoot, { runlarkBin, serverBin, version, target, profile, gitSha }) {
    const pluginSrcRoot = join(REPO_ROOT, "packages", "dsh-plugin");
    const workflowSrcRoot = join(REPO_ROOT, "workflows", "solo-default");

    copyChecked(join(REPO_ROOT, "LICENSE"), join(stageRoot, "LICENSE"));
    copyChecked(join(REPO_ROOT, "README.md"), join(stageRoot, "README.md"));
    copyChecked(join(REPO_ROOT, "docs", "running.md"), join(stageRoot, "RUNNING.md"));

    copyChecked(runlarkBin, join(stageRoot, "bin", "runlark"), 0o755);
    copyChecked(serverBin, join(stageRoot, "bin", "runlark-server"), 0o755);

    for (const file of WORKFLOW_FILES) {
        const src = join(workflowSrcRoot, file);
        assertExists(src, `workflow file ${file}`);
        copyChecked(src, join(stageRoot, "workflows", "solo-default", file));
    }

    const pluginDest = join(stageRoot, "dsh-plugin");
    const sourcePkg = JSON.parse(readFileSync(join(pluginSrcRoot, "package.json"), "utf8"));
    mkdirSync(pluginDest, { recursive: true });
    writeFileSync(
        join(pluginDest, "package.json"),
        `${JSON.stringify(buildRuntimePluginPackageJson(sourcePkg), null, 2)}\n`,
        "utf8",
    );
    copyChecked(join(pluginSrcRoot, "cordis.patch.yml"), join(pluginDest, "cordis.patch.yml"));
    copyChecked(
        join(pluginSrcRoot, "patches", PATCH_FILE),
        join(pluginDest, "patches", PATCH_FILE),
    );

    const hostSrcDir = join(pluginSrcRoot, "src");
    for (const entry of readdirSync(hostSrcDir, { withFileTypes: true })) {
        if (entry.isFile() && entry.name.endsWith(".ts")) {
            copyChecked(join(hostSrcDir, entry.name), join(pluginDest, "src", entry.name));
        }
    }

    const clientJs = join(pluginSrcRoot, "dist", "client.js");
    const clientCss = join(pluginSrcRoot, "dist", "client.css");
    assertExists(clientJs, "prebuilt DSH client bundle");
    copyChecked(clientJs, join(pluginDest, "dist", "client.js"));
    if (existsSync(clientCss)) {
        copyChecked(clientCss, join(pluginDest, "dist", "client.css"));
    }

    copyChecked(
        join(REPO_ROOT, "scripts", "setup-dsh-profile.mjs"),
        join(stageRoot, "scripts", "setup-dsh-profile.mjs"),
        0o755,
    );
    copyChecked(
        join(REPO_ROOT, "scripts", "apply-dsh-patch.mjs"),
        join(stageRoot, "scripts", "apply-dsh-patch.mjs"),
        0o755,
    );

    const stagedFiles = listFilesRecursive(stageRoot).map((relPath) => {
        const full = join(stageRoot, relPath);
        return {
            path: relPath,
            sha256: sha256File(full),
            bytes: statSync(full).size,
        };
    });

    const manifest = {
        schema_version: 1,
        name: "runlark",
        version,
        license: "Apache-2.0",
        target,
        profile,
        git_sha: gitSha,
        dsh_version: DSH_VERSION,
        files: stagedFiles,
    };
    const manifestPath = join(stageRoot, "manifest.json");
    writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`, "utf8");

    const allWithManifest = listFilesRecursive(stageRoot);
    const sums = allWithManifest
        .map((relPath) => `${sha256File(join(stageRoot, relPath))}  ${relPath}`)
        .join("\n");
    writeFileSync(join(stageRoot, "SHA256SUMS"), `${sums}\n`, "utf8");

    return manifest;
}

function main() {
    const {
        profile,
        skipBuild,
        outDir,
        version: versionArg,
        target: targetArg,
        jsonMode,
    } = parseArgs(process.argv.slice(2));
    const version = resolveVersion(versionArg);
    const target = resolveTargetTriple(targetArg);
    const gitSha = readGitSha();
    const buildStdio = jsonMode ? ["ignore", "pipe", "pipe"] : "inherit";

    if (!skipBuild) {
        const cargoArgs = ["build", "--bin", "runlark", "--bin", "runlark-server"];
        if (profile === "release") {
            cargoArgs.splice(1, 0, "--release");
        }
        execFileSync("cargo", cargoArgs, { cwd: REPO_ROOT, stdio: buildStdio });
        execFileSync(process.execPath, [join(REPO_ROOT, "scripts", "build-dsh-client.mjs")], {
            cwd: REPO_ROOT,
            stdio: buildStdio,
        });
    }

    const runlarkBin = join(REPO_ROOT, "target", profile, "runlark");
    const serverBin = join(REPO_ROOT, "target", profile, "runlark-server");
    assertExists(runlarkBin, "runlark binary");
    assertExists(serverBin, "runlark-server binary");
    assertExists(join(REPO_ROOT, "LICENSE"), "LICENSE");
    assertExists(join(REPO_ROOT, "README.md"), "README.md");
    assertExists(join(REPO_ROOT, "docs", "running.md"), "docs/running.md");
    assertExists(
        join(REPO_ROOT, "packages", "dsh-plugin", "patches", PATCH_FILE),
        "DSH controller patch",
    );
    assertExists(
        join(REPO_ROOT, "packages", "dsh-plugin", "dist", "client.js"),
        "DSH client UI bundle",
    );

    const validatedRaw = execFileSync(
        runlarkBin,
        ["plugin", "validate", join(REPO_ROOT, "workflows", "solo-default"), "--json"],
        { cwd: REPO_ROOT, encoding: "utf8" },
    );
    const validated = JSON.parse(validatedRaw);
    if (validated?.validation?.verdict !== "pass") {
        throw new Error(`Bundled solo-default workflow failed validation: ${validatedRaw}`);
    }

    const bundleName = `runlark-${version}-${target}`;
    const stageParent = mkdtempSync(join(tmpdir(), "runlark-package-"));
    const stageRoot = join(stageParent, bundleName);
    mkdirSync(stageRoot, { recursive: true });
    mkdirSync(outDir, { recursive: true });

    const archiveName = `${bundleName}.tar.gz`;
    const archivePath = join(outDir, archiveName);
    const checksumPath = `${archivePath}.sha256`;

    try {
        const manifest = stageBundle(stageRoot, {
            runlarkBin,
            serverBin,
            version,
            target,
            profile,
            gitSha,
        });

        execFileSync("tar", ["-czf", archivePath, "-C", stageParent, bundleName], {
            env: { ...process.env, COPYFILE_DISABLE: "1" },
            stdio: buildStdio,
        });

        const archiveSha256 = sha256File(archivePath);
        const archiveBytes = statSync(archivePath).size;
        writeFileSync(checksumPath, `${archiveSha256}  ${archiveName}\n`, "utf8");

        const summary = {
            bundle_name: bundleName,
            version,
            target,
            profile,
            git_sha: gitSha,
            archive_path: archivePath,
            checksum_path: checksumPath,
            archive_sha256: archiveSha256,
            archive_bytes: archiveBytes,
            file_count: manifest.files.length,
        };

        if (jsonMode) {
            console.log(JSON.stringify(summary, null, 2));
        } else {
            console.log(
                `Packaged ${archiveName} (${archiveBytes} bytes, sha256=${archiveSha256}) -> ${archivePath}`,
            );
            console.log(`Checksum written to ${checksumPath}`);
        }
    } finally {
        rmSync(stageParent, { recursive: true, force: true });
    }
}

main();
