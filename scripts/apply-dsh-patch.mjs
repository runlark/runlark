#!/usr/bin/env node
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const EXPECTED_VERSION = "0.2.2-rc.3";
const MARKER = "sessionCommandInterceptor";
const ROOT_DIR = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const PATCH_FILE = `@deepseek-ai+dsh-api-session-controller@${EXPECTED_VERSION}.patch`;

function resolvePatchPath(rootDir) {
    const candidates = [
        join(rootDir, "packages", "dsh-plugin", "patches", PATCH_FILE),
        join(rootDir, "dsh-plugin", "patches", PATCH_FILE),
    ];
    for (const candidate of candidates) {
        if (existsSync(candidate)) {
            return candidate;
        }
    }
    throw new Error(`DSH session controller patch not found (checked: ${candidates.join(", ")})`);
}

const PATCH_PATH = resolvePatchPath(ROOT_DIR);

function parseArgs(argv) {
    let targetDir = ROOT_DIR;
    let ifPresent = false;
    let checkOnly = false;
    for (let i = 0; i < argv.length; i += 1) {
        const arg = argv[i];
        if (arg === "--if-present") {
            ifPresent = true;
        } else if (arg === "--check") {
            checkOnly = true;
        } else if (arg === "--target" && i + 1 < argv.length) {
            targetDir = resolve(argv[i + 1]);
            i += 1;
        } else {
            throw new Error(`Unknown argument: ${arg}`);
        }
    }
    return { targetDir, ifPresent, checkOnly };
}

function isPatched(pkgDir) {
    const runtimeFile = join(pkgDir, "lib", "index.js");
    const typesFile = join(pkgDir, "lib", "types", "commands.d.ts");
    const indexTypesFile = join(pkgDir, "lib", "types", "index.d.ts");
    if (!existsSync(runtimeFile) || !existsSync(typesFile) || !existsSync(indexTypesFile)) {
        return false;
    }
    const runtime = readFileSync(runtimeFile, "utf8");
    const types = readFileSync(typesFile, "utf8");
    const indexTypes = readFileSync(indexTypesFile, "utf8");
    return (
        runtime.includes(MARKER) &&
        runtime.includes("executePrompt(request, source)") &&
        runtime.includes("executeCancel(request)") &&
        types.includes("interface SessionCommandInterceptor") &&
        indexTypes.includes("SessionCommandInterceptor")
    );
}

const { targetDir, ifPresent, checkOnly } = parseArgs(process.argv.slice(2));
const pkgDir = join(targetDir, "node_modules", "@deepseek-ai", "dsh-api-session-controller");

if (!existsSync(pkgDir)) {
    if (ifPresent) {
        process.exit(0);
    }
    throw new Error(`Target package not found: ${pkgDir}`);
}

const pkgJson = JSON.parse(readFileSync(join(pkgDir, "package.json"), "utf8"));
if (pkgJson.version !== EXPECTED_VERSION) {
    throw new Error(
        `Expected @deepseek-ai/dsh-api-session-controller@${EXPECTED_VERSION}, found ${pkgJson.version}`,
    );
}

if (isPatched(pkgDir)) {
    console.log(
        `Verified @deepseek-ai/dsh-api-session-controller@${EXPECTED_VERSION} is patched at ${pkgDir}.`,
    );
    process.exit(0);
}

if (checkOnly) {
    console.error(
        `ERROR: @deepseek-ai/dsh-api-session-controller@${EXPECTED_VERSION} is NOT patched at ${pkgDir}.`,
    );
    process.exit(1);
}

const patchContent = readFileSync(PATCH_PATH, "utf8");
execFileSync("patch", ["-p1", "--batch", "--no-backup-if-mismatch"], {
    cwd: pkgDir,
    input: patchContent,
    stdio: ["pipe", "inherit", "inherit"],
});

if (!isPatched(pkgDir)) {
    throw new Error(`Patch applied to ${pkgDir}, but verification markers are missing.`);
}

console.log(
    `Applied SessionCommandInterceptor patch to @deepseek-ai/dsh-api-session-controller@${EXPECTED_VERSION} at ${pkgDir}.`,
);
