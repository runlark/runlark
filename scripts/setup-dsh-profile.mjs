#!/usr/bin/env node
import { execFileSync } from "node:child_process";
import { cpSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const SCRIPT_DIR = dirname(fileURLToPath(import.meta.url));
const ROOT_DIR = resolve(SCRIPT_DIR, "..");
const DSH_VERSION = "0.2.2-rc.3";

function resolvePluginDir(rootDir) {
    const candidates = [join(rootDir, "packages", "dsh-plugin"), join(rootDir, "dsh-plugin")];
    for (const candidate of candidates) {
        if (existsSync(join(candidate, "package.json"))) {
            return candidate;
        }
    }
    throw new Error(`@runlark/dsh-plugin directory not found (checked: ${candidates.join(", ")})`);
}

function validateDaemonUrl(rawUrl) {
    let parsed;
    try {
        parsed = new URL(rawUrl);
    } catch {
        throw new Error(`Invalid --daemon-url (must be an http/https URL): ${rawUrl}`);
    }
    if (parsed.protocol !== "http:" && parsed.protocol !== "https:") {
        throw new Error(`Invalid --daemon-url protocol '${parsed.protocol}' (expected http/https)`);
    }
    if (/['"\r\n\\]/.test(rawUrl)) {
        throw new Error(`Invalid characters in --daemon-url: ${rawUrl}`);
    }
    return parsed.origin + parsed.pathname.replace(/\/+$/, "");
}

function parseArgs(argv) {
    const dshHome = process.env.DSH_HOME || join(homedir(), ".dsh");
    let profile = "runlark";
    let daemonUrl = "http://127.0.0.1:9876";
    let installDeps = true;

    for (let i = 0; i < argv.length; i += 1) {
        const arg = argv[i];
        if (arg === "--help" || arg === "-h") {
            console.log(
                [
                    "Usage: node scripts/setup-dsh-profile.mjs [options]",
                    "",
                    "Options:",
                    "  --profile <name>       DSH profile name (default: runlark)",
                    "  --daemon-url <url>     Runlark daemon URL (default: http://127.0.0.1:9876)",
                    "  --no-install           Write profile files without running pnpm install",
                    "  -h, --help             Show this help message",
                    "",
                    "Environment:",
                    "  DSH_HOME               Override DSH home directory (default: ~/.dsh)",
                ].join("\n"),
            );
            process.exit(0);
        } else if (arg === "--profile" && i + 1 < argv.length) {
            profile = argv[i + 1];
            i += 1;
        } else if (arg === "--daemon-url" && i + 1 < argv.length) {
            daemonUrl = argv[i + 1];
            i += 1;
        } else if (arg === "--no-install") {
            installDeps = false;
        } else {
            throw new Error(`Unknown argument: ${arg}`);
        }
    }

    if (!/^[a-zA-Z0-9][a-zA-Z0-9._-]*$/.test(profile) || profile.includes("..")) {
        throw new Error(`Invalid --profile name: ${profile}`);
    }

    return { dshHome, profile, daemonUrl: validateDaemonUrl(daemonUrl), installDeps };
}

const { dshHome, profile, daemonUrl, installDeps } = parseArgs(process.argv.slice(2));
const PLUGIN_DIR = resolvePluginDir(ROOT_DIR);
const buildScript = join(ROOT_DIR, "scripts", "build-dsh-client.mjs");
if (existsSync(buildScript)) {
    execFileSync(process.execPath, [buildScript], {
        stdio: "inherit",
    });
} else if (!existsSync(join(PLUGIN_DIR, "dist", "client.js"))) {
    throw new Error(
        `Prebuilt DSH client bundle missing at ${join(PLUGIN_DIR, "dist", "client.js")}`,
    );
}
const profileDir = join(dshHome, "profiles", profile);
mkdirSync(profileDir, { recursive: true });

const pkgJson = {
    name: `dsh-profile-${profile}`,
    private: true,
    dependencies: {
        "@deepseek-ai/dsh-api-session-controller": DSH_VERSION,
        "@runlark/dsh-plugin": `link:${PLUGIN_DIR}`,
    },
    dsh: {
        profile: {
            bundles: ["@deepseek-ai/dsh-base", "@deepseek-ai/dsh-web-app"],
        },
    },
};
writeFileSync(join(profileDir, "package.json"), `${JSON.stringify(pkgJson, null, 2)}\n`, "utf8");

const workspaceYaml = [
    "packages:",
    "  - .",
    "",
    "nodeLinker: hoisted",
    "autoInstallPeers: false",
    "patchedDependencies:",
    `  "@deepseek-ai/dsh-api-session-controller@${DSH_VERSION}": "./patches/@deepseek-ai+dsh-api-session-controller@${DSH_VERSION}.patch"`,
    "",
].join("\n");
writeFileSync(join(profileDir, "pnpm-workspace.yaml"), workspaceYaml, "utf8");

const patchesDir = join(profileDir, "patches");
mkdirSync(patchesDir, { recursive: true });
const patchName = `@deepseek-ai+dsh-api-session-controller@${DSH_VERSION}.patch`;
cpSync(join(PLUGIN_DIR, "patches", patchName), join(patchesDir, patchName));

const patchOverlay = readFileSync(join(PLUGIN_DIR, "cordis.patch.yml"), "utf8").replace(
    "http://127.0.0.1:9876",
    () => daemonUrl,
);
writeFileSync(join(profileDir, "cordis.patch.yml"), patchOverlay, "utf8");

if (installDeps) {
    const pnpmCmd = existsSync("/opt/homebrew/bin/pnpm") ? "/opt/homebrew/bin/pnpm" : "pnpm";
    execFileSync(pnpmCmd, ["install", "--no-frozen-lockfile"], {
        cwd: profileDir,
        stdio: "inherit",
    });
    execFileSync(
        process.execPath,
        [join(SCRIPT_DIR, "apply-dsh-patch.mjs"), "--target", profileDir],
        {
            stdio: "inherit",
        },
    );
}

console.log(`Configured Runlark DSH profile '${profile}' at ${profileDir}.`);
