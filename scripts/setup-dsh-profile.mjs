#!/usr/bin/env node
import { execFileSync } from "node:child_process";
import { cpSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const PLUGIN_DIR = join(REPO_ROOT, "packages", "dsh-plugin");
const DSH_VERSION = "0.2.2-rc.3";

function parseArgs(argv) {
    const dshHome = process.env.DSH_HOME || join(homedir(), ".dsh");
    let profile = "runlark";
    let daemonUrl = "http://127.0.0.1:9876";
    let installDeps = true;

    for (let i = 0; i < argv.length; i += 1) {
        const arg = argv[i];
        if (arg === "--profile" && i + 1 < argv.length) {
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

    return { dshHome, profile, daemonUrl, installDeps };
}

const { dshHome, profile, daemonUrl, installDeps } = parseArgs(process.argv.slice(2));
execFileSync(process.execPath, [join(REPO_ROOT, "scripts", "build-dsh-client.mjs")], {
    stdio: "inherit",
});
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
    daemonUrl,
);
writeFileSync(join(profileDir, "cordis.patch.yml"), patchOverlay, "utf8");

if (installDeps) {
    const pnpmCmd = existsSync("/opt/homebrew/bin/pnpm") ? "/opt/homebrew/bin/pnpm" : "pnpm";
    execFileSync(pnpmCmd, ["install", "--no-frozen-lockfile"], {
        cwd: profileDir,
        stdio: "inherit",
    });
    execFileSync(
        "node",
        [join(REPO_ROOT, "scripts", "apply-dsh-patch.mjs"), "--target", profileDir],
        {
            stdio: "inherit",
        },
    );
}

console.log(`Configured Runlark DSH profile '${profile}' at ${profileDir}.`);
