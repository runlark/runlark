import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "node:test";

const script = fileURLToPath(new URL("./check-commits.mjs", import.meta.url));

const check = (message) => {
    const cwd = mkdtempSync(join(tmpdir(), "runlark-commit-"));

    try {
        const file = join(cwd, "COMMIT_EDITMSG");

        writeFileSync(file, message);

        return spawnSync(process.execPath, [script, "--message", file], { encoding: "utf8" });
    } finally {
        rmSync(cwd, { recursive: true, force: true });
    }
};

for (const message of [
    "feat: add task creation\n",
    "fix(local): recover interrupted run\n",
    "feat(api)!: change command contract\n\nBREAKING CHANGE: command revision is required\n",
    "docs: уточнить правила коммитов\n",
    "chore(deps): refresh lockfile\n\nKeep the existing dependency policy.\n\nRefs: #1\n",
    "CI: check hooks\n",
    "revert: undo broken adapter\n",
]) {
    test(`accepts Conventional Commit: ${message.split("\n")[0]}`, () => {
        const result = check(message);

        assert.equal(result.status, 0, result.stderr);
    });
}

for (const message of [
    "",
    "update docs\n",
    "feat: \n",
    "fix:    \n",
    "feat(): add task\n",
    "feat( ): add task\n",
    "fix:missing space\n",
    "fixup! fix: recover run\n",
    "feat: add task\nBody without a blank line\n",
]) {
    test(`refuses invalid commit: ${JSON.stringify(message)}`, () => {
        const result = check(message);

        assert.equal(result.status, 1);
        assert.match(result.stderr, /Conventional Commits/);
    });
}
