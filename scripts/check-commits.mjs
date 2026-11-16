#!/usr/bin/env node
import { execFileSync, spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";

const header =
    /^(feat|fix|docs|style|refactor|perf|test|build|ci|chore|revert)(?:\(([^()\r\n]+)\))?!?: \S[^\r\n]*$/i;
const oid = /^(?:[a-f0-9]{40}|[a-f0-9]{64})$/i;
const git = (...args) => execFileSync("git", args, { encoding: "utf8" });

const validate = (message, identity) => {
    const lines = message.split(/\r?\n/);
    const match = header.exec(lines[0]);

    if (!match || (match[2] !== undefined && !match[2].trim())) {
        throw new Error(
            `Conventional Commits required for ${identity}: ${JSON.stringify(lines[0])}\n` +
                "Expected type(scope)!: description; scope and ! are optional.\n" +
                "Types: feat, fix, docs, style, refactor, perf, test, build, ci, chore, revert.",
        );
    }

    if (lines[1]?.trim()) {
        throw new Error(`Conventional Commits requires a blank line before the body: ${identity}`);
    }
};

// Tags may refer to commits, trees or blobs; only reachable commits have messages to check.
const asCommit = (value) => {
    const type = git("cat-file", "-t", value).trim();

    if (type === "commit") {
        return value;
    }

    if (type !== "tag") {
        return null;
    }

    const peeled = spawnSync("git", ["rev-parse", "--verify", "--quiet", `${value}^{commit}`], {
        encoding: "utf8",
    });

    if (peeled.error) {
        throw peeled.error;
    }

    return peeled.status === 0 ? peeled.stdout.trim() : null;
};

const checkPush = (remote) => {
    const tips = new Set();
    // Existing remote history predates this rule. Check only commits being published now.
    const published = new Set(
        git("for-each-ref", "--format=%(objectname)", `refs/remotes/${remote}/`)
            .trim()
            .split(/\s+/)
            .filter(Boolean),
    );

    for (const line of readFileSync(0, "utf8").split(/\r?\n/).filter(Boolean)) {
        const fields = line.split(/\s+/);

        if (fields.length !== 4 || !oid.test(fields[1]) || !oid.test(fields[3])) {
            throw new Error("Invalid Git pre-push ref update; refusing to skip commit validation");
        }

        const [, localOid, , remoteOid] = fields;

        if (/^0+$/.test(localOid)) {
            continue;
        }

        const tip = asCommit(localOid);

        if (tip) {
            tips.add(tip);
        }

        if (!/^0+$/.test(remoteOid)) {
            const previous = asCommit(remoteOid);

            if (previous) {
                published.add(previous);
            }
        }
    }

    if (!tips.size) {
        return;
    }

    const commits = git("rev-list", "--reverse", ...tips, "--not", ...published)
        .trim()
        .split(/\s+/)
        .filter(Boolean);

    for (const commit of commits) {
        validate(git("show", "-s", "--format=%B", commit), commit.slice(0, 12));
    }

    console.log(`check-commits: verified ${commits.length} outgoing commit(s)`);
};

try {
    const [mode, value, ...extra] = process.argv.slice(2);

    if (!value || extra.length || !["--message", "--push"].includes(mode)) {
        throw new Error("usage: check-commits.mjs --message FILE | --push REMOTE");
    }

    if (mode === "--message") {
        validate(readFileSync(value, "utf8"), value);
    } else {
        checkPush(value);
    }
} catch (error) {
    console.error(`check-commits: ${error.message}`);
    process.exitCode = 1;
}
