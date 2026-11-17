#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const REPO_ROOT = resolve(fileURLToPath(new URL("..", import.meta.url)));

const IGNORE_DIRS = new Set([
    ".generated",
    ".git",
    ".impeccable",
    "dist",
    "node_modules",
    "storybook-static",
    "target",
]);

function hasMatchingFiles(dir, pattern) {
    if (!existsSync(dir)) {
        return false;
    }

    for (const entry of readdirSync(dir, { withFileTypes: true })) {
        if (entry.isDirectory()) {
            if (IGNORE_DIRS.has(entry.name)) {
                continue;
            }
            if (hasMatchingFiles(join(dir, entry.name), pattern)) {
                return true;
            }
            continue;
        }
        if (entry.isFile() && pattern.test(entry.name)) {
            return true;
        }
    }

    return false;
}

function runChecked(command, args, label) {
    const result = spawnSync(command, args, {
        cwd: REPO_ROOT,
        encoding: "utf8",
    });

    if (result.status !== 0) {
        if (result.stdout) {
            process.stdout.write(result.stdout);
        }
        if (result.stderr) {
            process.stderr.write(result.stderr);
        }
        throw new Error(`${label} failed with exit code ${result.status ?? 1}`);
    }

    return result;
}

function localBin(name) {
    return join(REPO_ROOT, "node_modules", ".bin", name);
}

function assertRequiredFiles() {
    const required = [
        "feod.profile.md",
        "feod.config.json",
        "oxlint.config.ts",
        "web/tsconfig.ui.json",
        "packages/dsh-plugin/tsconfig.json",
    ];

    for (const rel of required) {
        if (!existsSync(join(REPO_ROOT, rel))) {
            throw new Error(`Missing required UI gate file: ${rel}`);
        }
    }
}

function checkTypeScript() {
    const tsc = localBin("tsc");

    runChecked(
        tsc,
        ["-p", "packages/dsh-plugin/tsconfig.json", "--noEmit"],
        "TypeScript check (packages/dsh-plugin)",
    );

    if (existsSync(join(REPO_ROOT, "packages/dsh-plugin/src/ui/tsconfig.json"))) {
        runChecked(
            tsc,
            ["-p", "packages/dsh-plugin/src/ui/tsconfig.json", "--noEmit"],
            "TypeScript check (packages/dsh-plugin UI)",
        );
    }

    runChecked(
        tsc,
        [
            "--noEmit",
            "--target",
            "ES2022",
            "--module",
            "ESNext",
            "--moduleResolution",
            "bundler",
            "--types",
            "node",
            "--strict",
            "--skipLibCheck",
            "oxlint.config.ts",
        ],
        "TypeScript check (oxlint.config.ts)",
    );

    const webUiConfig = JSON.parse(readFileSync(join(REPO_ROOT, "web/tsconfig.ui.json"), "utf8"));
    if (webUiConfig?.compilerOptions?.strict !== true) {
        throw new Error("web/tsconfig.ui.json must enforce compilerOptions.strict = true");
    }
    if (webUiConfig?.compilerOptions?.jsx !== "react-jsx") {
        throw new Error('web/tsconfig.ui.json must set compilerOptions.jsx = "react-jsx"');
    }

    const webDir = join(REPO_ROOT, "web");
    if (hasMatchingFiles(webDir, /\.(?:[cm]?ts|tsx)$/)) {
        const webTsConfig = join(webDir, "tsconfig.json");
        if (!existsSync(webTsConfig)) {
            throw new Error(
                "web/ contains TypeScript sources; create web/tsconfig.json extending ./tsconfig.ui.json",
            );
        }
        runChecked(tsc, ["-p", "web/tsconfig.json", "--noEmit"], "TypeScript check (web)");
    }
}

function checkOxlint() {
    const oxlint = localBin("oxlint");
    const targets = ["packages/dsh-plugin"];

    if (hasMatchingFiles(join(REPO_ROOT, "web"), /\.(?:[cm]?[jt]s|[jt]sx)$/)) {
        targets.push("web");
    }

    runChecked(oxlint, ["--config", "oxlint.config.ts", ...targets], "Oxlint UI/DSH gate");
}

function checkFeod() {
    const feodCli = join(REPO_ROOT, ".agents/skills/feod/scripts/feod.mjs");
    const result = spawnSync(process.execPath, [feodCli, "info", "--json"], {
        cwd: REPO_ROOT,
        encoding: "utf8",
    });

    let info;
    try {
        info = JSON.parse(result.stdout);
    } catch {
        throw new Error(
            `feod info --json did not return valid JSON:\n${result.stderr || result.stdout}`,
        );
    }

    if (info.error) {
        throw new Error(`FEOD configuration error: ${info.error}`);
    }
    if (info.configSource !== "feod.config.json") {
        throw new Error(`Expected FEOD configSource 'feod.config.json', got ${info.configSource}`);
    }
    if (info.profile?.path !== "feod.profile.md") {
        throw new Error("Expected feod.profile.md to be loaded at repository root");
    }
    if (info.profile?.values?.["common.allowIndex"] !== "false") {
        throw new Error("feod.profile.md must lock common.allowIndex = false");
    }

    const webSrcExists = existsSync(join(REPO_ROOT, "web/src"));
    const findings = Array.isArray(info.findings) ? info.findings : [];
    const unexpected = findings.filter((finding) => {
        if (!webSrcExists && finding.code === "root-missing") {
            return false;
        }
        return finding.severity === "error" || finding.severity === "warn";
    });

    if (unexpected.length > 0) {
        const formatted = unexpected
            .map((item) => `[${item.severity}] ${item.code}: ${item.message}`)
            .join("\n");
        throw new Error(`FEOD layout findings detected:\n${formatted}`);
    }
}

function checkImpeccable() {
    const impeccable = join(REPO_ROOT, ".agents/skills/impeccable/scripts/impeccable");
    const targets = ["web"];
    const dshUiDir = join(REPO_ROOT, "packages/dsh-plugin/src/ui");
    if (existsSync(dshUiDir)) {
        targets.push("packages/dsh-plugin/src/ui");
    }

    const result = spawnSync(impeccable, ["detect", "--json", ...targets], {
        cwd: REPO_ROOT,
        encoding: "utf8",
    });

    if (result.status !== 0) {
        throw new Error(
            `impeccable detect failed (exit ${result.status}):\n${result.stdout}\n${result.stderr}`,
        );
    }

    const parsed = JSON.parse(result.stdout || "[]");
    if (Array.isArray(parsed) && parsed.length > 0) {
        throw new Error(`impeccable detect reported findings:\n${result.stdout}`);
    }
}

assertRequiredFiles();
checkTypeScript();
checkOxlint();
checkFeod();
checkImpeccable();
console.log(
    "UI gates passed (TypeScript strict, Oxlint React/a11y/FEOD, FEOD layout, Impeccable detect).",
);
