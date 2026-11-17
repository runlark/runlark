#!/usr/bin/env node
// Product sources cannot silently enter a documentation/tooling-only gate.
import { execFileSync } from "node:child_process";

const files = execFileSync("git", ["ls-files", "-z"], { encoding: "utf8" })
    .split("\0")
    .filter(Boolean);
const forbidden = files.filter((file) => {
    if (file.startsWith(".agents/skills/")) {
        return false;
    }

    if (file.startsWith("scripts/") && /\.mjs$/.test(file)) {
        return false;
    }

    if (
        file.startsWith("wit/") &&
        !file.slice("wit/".length).includes("/") &&
        /\.wit$/.test(file)
    ) {
        return false;
    }

    if (/^workflows\/[a-z0-9-]+\/[^/]+\.(?:wasm|wat)$/.test(file)) {
        return false;
    }

    if (
        (file.startsWith("web/") || file.startsWith("packages/dsh-plugin/")) &&
        /\.(?:tsx|css)$/.test(file)
    ) {
        return false;
    }

    return (
        /(?:^|\/)(?:Dockerfile)$/.test(file) ||
        /\.(?:[cm]?js|[jt]sx|sh|bash|wasm|wat|wit|py|go|lean|tsp|css|scss|html)$/.test(file)
    );
});

if (forbidden.length) {
    console.error("Product source requires its real gates before entering this bootstrap:");
    console.error(forbidden.join("\n"));
    console.error(
        "Follow docs/gates.md; integrate compiler, architecture, tests and real smoke together.",
    );
    process.exitCode = 1;
}
