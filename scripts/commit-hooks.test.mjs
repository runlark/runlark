import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { chmodSync, copyFileSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "node:test";

const root = fileURLToPath(new URL("../", import.meta.url));
const env = (cwd) => ({
    ...process.env,
    MISE_CONFIG_FILE: join(cwd, "empty-global.toml"),
    MISE_TRUSTED_CONFIG_PATHS: cwd,
});
const git = (cwd, ...args) =>
    execFileSync("git", args, {
        cwd,
        env: env(cwd),
        encoding: "utf8",
        stdio: ["ignore", "pipe", "pipe"],
    }).trim();
const run = (cwd, ...args) => spawnSync("git", args, { cwd, env: env(cwd), encoding: "utf8" });
const commit = (cwd, message) => git(cwd, "commit", "--quiet", "--allow-empty", "-m", message);

const fixture = (t) => {
    const dir = mkdtempSync(join(tmpdir(), "runlark-hook-"));
    const local = join(dir, "local");
    const remote = join(dir, "remote.git");

    t.after(() => rmSync(dir, { recursive: true, force: true }));
    mkdirSync(local);
    writeFileSync(join(local, "empty-global.toml"), "");
    git(dir, "init", "--bare", "--quiet", remote);
    git(local, "init", "--quiet", "-b", "main");
    git(local, "config", "user.name", "Hook test");
    git(local, "config", "user.email", "hooks@example.invalid");
    git(local, "config", "commit.gpgsign", "false");
    git(local, "config", "core.hooksPath", ".githooks");
    commit(local, "Historical bootstrap before the policy");
    git(local, "remote", "add", "origin", remote);
    git(local, "push", "--quiet", "origin", "main");

    const install = () => {
        mkdirSync(join(local, ".githooks"));
        mkdirSync(join(local, "scripts"));
        // Hooks resolve pinned tools from this temporary project, without global defaults.
        copyFileSync(join(root, "mise.toml"), join(local, "mise.toml"));
        copyFileSync(
            join(root, "scripts/check-commits.mjs"),
            join(local, "scripts/check-commits.mjs"),
        );

        for (const name of ["pre-push", "commit-msg"]) {
            copyFileSync(join(root, ".githooks", name), join(local, ".githooks", name));
            chmodSync(join(local, ".githooks", name), 0o755);
        }

        // The fixture's real check can fail independently of commit validation.
        writeFileSync(join(local, "justfile"), "check:\n    @test -f verification-passed\n");
        writeFileSync(join(local, "verification-passed"), "passed\n");
    };

    return { local, remote, install };
};

test("commit-msg rejects an invalid message before creating a commit", (t) => {
    const { local, install } = fixture(t);
    const before = git(local, "rev-parse", "HEAD");

    install();

    const invalid = run(local, "commit", "--quiet", "--allow-empty", "-m", "update hooks");

    assert.notEqual(invalid.status, 0);
    assert.match(invalid.stderr, /Conventional Commits/);
    assert.equal(git(local, "rev-parse", "HEAD"), before);
    commit(local, "chore(git): enable commit checks");
    assert.notEqual(git(local, "rev-parse", "HEAD"), before);
});

test("pre-push refuses an earlier invalid commit even when the tip is valid", (t) => {
    const { local, remote, install } = fixture(t);
    const before = git(remote, "rev-parse", "refs/heads/main");

    commit(local, "bad message");
    commit(local, "test: cover outgoing history");
    install();

    const result = run(local, "push", "origin", "main");

    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /Conventional Commits/);
    assert.match(result.stderr, /bad message/);
    assert.equal(git(remote, "rev-parse", "refs/heads/main"), before);
});

test("new branch accepts valid commits without rechecking published bootstrap history", (t) => {
    const { local, remote, install } = fixture(t);

    commit(local, "feat(board)!: separate stages from columns");
    install();

    const result = run(local, "push", "origin", "HEAD:refs/heads/feature");

    assert.equal(result.status, 0, result.stderr);
    assert.equal(git(remote, "rev-parse", "refs/heads/feature"), git(local, "rev-parse", "HEAD"));
});

test("pre-push checks every ref in an atomic multi-branch push", (t) => {
    const { local, remote, install } = fixture(t);
    const before = git(remote, "rev-parse", "refs/heads/main");

    commit(local, "docs: describe board");
    git(local, "switch", "--quiet", "-c", "invalid");
    commit(local, "unfinished changes");
    git(local, "switch", "--quiet", "main");
    install();

    const result = run(local, "push", "--atomic", "origin", "main", "invalid");

    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /unfinished changes/);
    assert.equal(git(remote, "rev-parse", "refs/heads/main"), before);
    assert.notEqual(run(remote, "rev-parse", "--verify", "refs/heads/invalid").status, 0);
});

test("a valid message still cannot bypass the full pre-push check", (t) => {
    const { local, remote, install } = fixture(t);
    const before = git(remote, "rev-parse", "refs/heads/main");

    commit(local, "fix: preserve required verification");
    install();
    rmSync(join(local, "verification-passed"));

    assert.notEqual(run(local, "push", "origin", "main").status, 0);
    assert.equal(git(remote, "rev-parse", "refs/heads/main"), before);
    writeFileSync(join(local, "verification-passed"), "passed\n");

    const result = run(local, "push", "origin", "main");

    assert.equal(result.status, 0, result.stderr);
    assert.equal(git(remote, "rev-parse", "refs/heads/main"), git(local, "rev-parse", "HEAD"));
});

test("deleting a remote branch does not treat the zero OID as a commit", (t) => {
    const { local, remote, install } = fixture(t);

    git(local, "push", "--quiet", "origin", "HEAD:refs/heads/obsolete");
    install();

    const result = run(local, "push", "origin", ":refs/heads/obsolete");

    assert.equal(result.status, 0, result.stderr);
    assert.notEqual(run(remote, "rev-parse", "--verify", "refs/heads/obsolete").status, 0);
});

test("an annotated tag cannot publish an invalid commit", (t) => {
    const { local, remote, install } = fixture(t);

    commit(local, "untyped release change");
    git(local, "-c", "tag.gpgsign=false", "tag", "--annotate", "v0.1.0", "-m", "release");
    install();

    const result = run(local, "push", "origin", "refs/tags/v0.1.0");

    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /untyped release change/);
    assert.notEqual(run(remote, "rev-parse", "--verify", "refs/tags/v0.1.0").status, 0);
});
