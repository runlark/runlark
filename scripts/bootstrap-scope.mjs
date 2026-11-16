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

    return (
        /(?:^|\/)(?:Cargo\.toml|Dockerfile|tsconfig[^/]*\.json)$/.test(file) ||
        /\.(?:rs|[cm]?[jt]sx?|css|scss|html|wasm|wit|sql|py|go|sh|bash|lean|tsp)$/.test(file)
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
