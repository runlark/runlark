import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "node:test";

const script = fileURLToPath(new URL("./bootstrap-scope.mjs", import.meta.url));

const check = (files) => {
    const cwd = mkdtempSync(join(tmpdir(), "runlark-scope-"));

    try {
        execFileSync("git", ["init", "--quiet"], { cwd });

        for (const file of files) {
            mkdirSync(dirname(join(cwd, file)), { recursive: true });
            writeFileSync(join(cwd, file), "bootstrap fixture\n");
        }

        execFileSync("git", ["add", "."], { cwd });

        return spawnSync(process.execPath, [script], { cwd, encoding: "utf8" });
    } finally {
        rmSync(cwd, { recursive: true, force: true });
    }
};

test("documentation, tooling and preserved vendor guidance pass", () => {
    const result = check(["README.md", "scripts/check.mjs", ".agents/skills/example/guide.ts"]);

    assert.equal(result.status, 0, result.stderr);
});

test("integrated languages (Rust, TypeScript, SQL) pass scope check", () => {
    const result = check([
        "crates/runlark-domain/src/lib.rs",
        "Cargo.toml",
        "packages/dsh-plugin/src/index.ts",
        "crates/runlark-local/migrations/0001_initial.sql",
    ]);

    assert.equal(result.status, 0, result.stderr);
});

for (const file of [
    "web/src/app.tsx",
    "images/check/Dockerfile",
    "plugins/result.wit",
    "bin/sample.wasm",
    "models/spec.lean",
    "services/backend.go",
    "scripts/deploy.py",
    "spec/api.tsp",
    "web/styles.css",
    "web/index.html",
]) {
    test(`unintegrated product source is refused: ${file}`, () => {
        const result = check(["README.md", file]);

        assert.equal(result.status, 1);
        assert.match(result.stderr, /requires its real gates/);
        assert.ok(result.stderr.includes(file));
    });
}
