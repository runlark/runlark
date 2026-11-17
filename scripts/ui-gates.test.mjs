import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import {
    copyFileSync,
    mkdtempSync,
    mkdirSync,
    readFileSync,
    realpathSync,
    rmSync,
    symlinkSync,
    writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "node:test";

const REPO_ROOT = resolve(fileURLToPath(new URL("..", import.meta.url)));
const FEOD_CLI = join(REPO_ROOT, ".agents/skills/feod/scripts/feod.mjs");
const IMPECCABLE_CLI = join(REPO_ROOT, ".agents/skills/impeccable/scripts/impeccable");
const OXLINT_BIN = join(REPO_ROOT, "node_modules/.bin/oxlint");
const TSC_BIN = join(REPO_ROOT, "node_modules/.bin/tsc");

function withTempWorkspace(files, fn) {
    const dir = realpathSync(mkdtempSync(join(tmpdir(), "runlark-ui-gate-")));

    try {
        for (const [rel, content] of Object.entries(files)) {
            const full = join(dir, rel);
            mkdirSync(dirname(full), { recursive: true });
            writeFileSync(full, content);
        }
        return fn(dir);
    } finally {
        rmSync(dir, { recursive: true, force: true });
    }
}

function setupFeodTemp(dir) {
    copyFileSync(join(REPO_ROOT, "feod.profile.md"), join(dir, "feod.profile.md"));
    copyFileSync(join(REPO_ROOT, "feod.config.json"), join(dir, "feod.config.json"));
    copyFileSync(join(REPO_ROOT, "oxlint.config.ts"), join(dir, "oxlint.config.ts"));
    symlinkSync(join(REPO_ROOT, "node_modules"), join(dir, "node_modules"), "dir");
}

test("FEOD profile and config pass on valid layout and catch common barrel/missing module index", () => {
    withTempWorkspace(
        {
            "web/src/app/index.tsx": "export const App = () => null;\n",
            "web/src/pages/board.tsx": "export const BoardPage = () => null;\n",
            "web/src/modules/task-flow/index.ts": "export const taskFlow = true;\n",
            "web/src/common/ui/BoardCard.tsx": "export const BoardCard = () => null;\n",
            "web/src/global/env.d.ts": "export {};\n",
        },
        (dir) => {
            setupFeodTemp(dir);
            const ok = spawnSync(process.execPath, [FEOD_CLI, "info", "--json", "--root", dir], {
                encoding: "utf8",
            });
            assert.equal(ok.status, 0, ok.stderr || ok.stdout);
            const parsed = JSON.parse(ok.stdout);
            assert.equal(parsed.configSource, "feod.config.json");
            assert.equal(parsed.profile?.path, "feod.profile.md");
            assert.deepEqual(parsed.findings, []);

            writeFileSync(join(dir, "web/src/common/index.ts"), "export {};\n");
            mkdirSync(join(dir, "web/src/modules/orphan-mod"), { recursive: true });
            writeFileSync(join(dir, "web/src/modules/orphan-mod/view.ts"), "export const x = 1;\n");

            const bad = spawnSync(process.execPath, [FEOD_CLI, "info", "--json", "--root", dir], {
                encoding: "utf8",
            });
            const badInfo = JSON.parse(bad.stdout);
            const codes = badInfo.findings.map((f) => f.code);
            assert.ok(codes.includes("common-index"), `expected common-index in ${codes}`);
            assert.ok(codes.includes("module-no-entry"), `expected module-no-entry in ${codes}`);
        },
    );
});

test("Oxlint enforces React hooks, jsx-a11y, FEOD layer imports, and Mantine/Reatom confinement", () => {
    withTempWorkspace(
        {
            "web/src/common/ui/ActionButton.tsx": [
                'import { Button } from "@mantine/core";',
                "export interface ActionButtonProps { readonly label: string }",
                "export const ActionButton = ({ label }: ActionButtonProps) => (",
                '  <Button type="button">{label}</Button>',
                ");",
            ].join("\n"),
            "web/src/modules/task-flow/index.ts": "export const taskFlowId = 'task-flow';\n",
        },
        (dir) => {
            setupFeodTemp(dir);
            const clean = spawnSync(
                OXLINT_BIN,
                ["--config", join(dir, "oxlint.config.ts"), join(dir, "web/src")],
                { cwd: dir, encoding: "utf8" },
            );
            assert.equal(clean.status, 0, clean.stdout + clean.stderr);

            // Violation 1: react/rules-of-hooks
            writeFileSync(
                join(dir, "web/src/modules/task-flow/BadHooks.tsx"),
                [
                    'import { useState } from "react";',
                    "export const BadHooks = ({ flag }: { flag: boolean }) => {",
                    "  if (flag) { useState(0); }",
                    "  return <div />;",
                    "};",
                ].join("\n"),
            );
            const hooksRes = spawnSync(
                OXLINT_BIN,
                [
                    "--config",
                    join(dir, "oxlint.config.ts"),
                    join(dir, "web/src/modules/task-flow/BadHooks.tsx"),
                ],
                { cwd: dir, encoding: "utf8" },
            );
            assert.notEqual(hooksRes.status, 0, "expected rules-of-hooks violation");
            assert.match(hooksRes.stdout + hooksRes.stderr, /rules-of-hooks/);

            // Violation 2: jsx-a11y
            writeFileSync(
                join(dir, "web/src/modules/task-flow/BadA11y.tsx"),
                'export const BadA11y = () => <img src="logo.png" />;\n',
            );
            const a11yRes = spawnSync(
                OXLINT_BIN,
                [
                    "--config",
                    join(dir, "oxlint.config.ts"),
                    join(dir, "web/src/modules/task-flow/BadA11y.tsx"),
                ],
                { cwd: dir, encoding: "utf8" },
            );
            assert.notEqual(a11yRes.status, 0, "expected jsx-a11y violation");
            assert.match(a11yRes.stdout + a11yRes.stderr, /alt-text/);

            // Violation 3: FEOD layer-imports (common -> modules)
            mkdirSync(join(dir, "web/src/common"), { recursive: true });
            writeFileSync(
                join(dir, "web/src/common/bad.ts"),
                'import { taskFlowId } from "@/modules/task-flow";\nexport const leaked = taskFlowId;\n',
            );
            const layerRes = spawnSync(
                OXLINT_BIN,
                ["--config", join(dir, "oxlint.config.ts"), join(dir, "web/src/common/bad.ts")],
                { cwd: dir, encoding: "utf8" },
            );
            assert.notEqual(layerRes.status, 0, "expected FEOD layer-imports violation");
            assert.match(layerRes.stdout + layerRes.stderr, /layer-imports/);

            // Violation 4: FEOD no-common-index (even inside common/ui)
            writeFileSync(join(dir, "web/src/common/ui/index.ts"), "export {};\n");
            const commonIdxRes = spawnSync(
                OXLINT_BIN,
                [
                    "--config",
                    join(dir, "oxlint.config.ts"),
                    join(dir, "web/src/common/ui/index.ts"),
                ],
                { cwd: dir, encoding: "utf8" },
            );
            assert.notEqual(commonIdxRes.status, 0, "expected FEOD no-common-index violation");
            assert.match(commonIdxRes.stdout + commonIdxRes.stderr, /no-common-index/);

            // Violation 5: FEOD confine (@mantine/* outside web/src/common/ui/**)
            writeFileSync(
                join(dir, "web/src/modules/task-flow/BadMantine.tsx"),
                'import { Box } from "@mantine/core";\nexport const BadMantine = () => <Box />;\n',
            );
            const mantineRes = spawnSync(
                OXLINT_BIN,
                [
                    "--config",
                    join(dir, "oxlint.config.ts"),
                    join(dir, "web/src/modules/task-flow/BadMantine.tsx"),
                ],
                { cwd: dir, encoding: "utf8" },
            );
            assert.notEqual(mantineRes.status, 0, "expected Mantine confinement violation");
            assert.match(mantineRes.stdout + mantineRes.stderr, /confine/);

            // Violation 6: FEOD confine (@reatom/jsx forbidden everywhere)
            mkdirSync(join(dir, "web/src/app"), { recursive: true });
            writeFileSync(
                join(dir, "web/src/app/BadJsx.tsx"),
                'import { mount } from "@reatom/jsx";\nexport const bad = mount;\n',
            );
            const reatomJsxRes = spawnSync(
                OXLINT_BIN,
                ["--config", join(dir, "oxlint.config.ts"), join(dir, "web/src/app/BadJsx.tsx")],
                { cwd: dir, encoding: "utf8" },
            );
            assert.notEqual(reatomJsxRes.status, 0, "expected @reatom/jsx confinement violation");
            assert.match(reatomJsxRes.stdout + reatomJsxRes.stderr, /confine/);
        },
    );
});

test("TypeScript strict UI config rejects implicit any in TSX", () => {
    const baseCfg = JSON.parse(readFileSync(join(REPO_ROOT, "web/tsconfig.ui.json"), "utf8"));
    withTempWorkspace(
        {
            "tsconfig.json": JSON.stringify({
                ...baseCfg,
                compilerOptions: {
                    ...baseCfg.compilerOptions,
                    paths: { "@/*": ["./src/*"] },
                },
                include: ["src/**/*"],
            }),
            "src/global/jsx.d.ts": [
                'declare module "react/jsx-runtime" {',
                "  export function jsx(type: unknown, props: unknown): unknown;",
                "  export function jsxs(type: unknown, props: unknown): unknown;",
                "}",
                "declare namespace JSX {",
                "  interface IntrinsicElements { div: { children?: unknown }; }",
                "}",
            ].join("\n"),
            "src/common/ui/Good.tsx":
                "export const Good = ({ label }: { readonly label: string }) => <div>{label}</div>;\n",
        },
        (dir) => {
            const ok = spawnSync(TSC_BIN, ["-p", join(dir, "tsconfig.json"), "--noEmit"], {
                encoding: "utf8",
            });
            assert.equal(ok.status, 0, ok.stdout + ok.stderr);

            writeFileSync(
                join(dir, "src/common/ui/Bad.tsx"),
                "export const Bad = (props) => <div>{props.label}</div>;\n",
            );
            const bad = spawnSync(TSC_BIN, ["-p", join(dir, "tsconfig.json"), "--noEmit"], {
                encoding: "utf8",
            });
            assert.notEqual(bad.status, 0, "expected strict TS error on implicit any");
            assert.match(bad.stdout + bad.stderr, /TS7006/);
        },
    );
});

test("Impeccable detect flags anti-patterns and accepts clean stylesheets", () => {
    withTempWorkspace(
        {
            "clean.css": ".board { font-family: Manrope, system-ui, sans-serif; }\n",
            "bad.css": ".card { font-family: Inter; border-left: 4px solid #edb830; }\n",
        },
        (dir) => {
            const clean = spawnSync(IMPECCABLE_CLI, ["detect", "--json", join(dir, "clean.css")], {
                encoding: "utf8",
            });
            assert.equal(clean.status, 0, clean.stderr);

            const bad = spawnSync(IMPECCABLE_CLI, ["detect", "--json", join(dir, "bad.css")], {
                encoding: "utf8",
            });
            assert.equal(bad.status, 2, bad.stderr);
            const findings = JSON.parse(bad.stdout);
            assert.ok(findings.length > 0);
        },
    );
});
