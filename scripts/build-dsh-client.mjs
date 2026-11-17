#!/usr/bin/env node
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import * as esbuild from "esbuild";

const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const ENTRY_FILE = join(REPO_ROOT, "packages", "dsh-plugin", "src", "ui", "index.ts");
const OUT_FILE = join(REPO_ROOT, "packages", "dsh-plugin", "dist", "client.js");
const OUT_CSS_FILE = join(REPO_ROOT, "packages", "dsh-plugin", "dist", "client.css");

const DSH_CLIENT_EXTERNALS = [
    "react",
    "react/jsx-runtime",
    "react-dom",
    "react-dom/client",
    "@deepseek-ai/cordis",
    "zod",
    "@tanstack/react-virtual",
    "@deepseek-ai/dsh-client-store",
    "@deepseek-ai/dsh-client-ui-slots",
    "@deepseek-ai/dsh-client-ui-primitives",
    "@deepseek-ai/dsh-client-ui-dockkit",
];

function buildStyleInjection(cssCode) {
    if (cssCode.trim().length === 0) {
        return "";
    }
    return [
        `const __runlarkStyleId = "@runlark/dsh-plugin/client.css";`,
        `if (typeof document !== "undefined" && document.querySelector('style[data-plugin-css=' + JSON.stringify(__runlarkStyleId) + ']') === null) {`,
        `  const __style = document.createElement("style");`,
        `  __style.dataset.plugin = "@runlark/dsh-plugin";`,
        `  __style.dataset.pluginCss = __runlarkStyleId;`,
        `  __style.textContent = ${JSON.stringify(cssCode)};`,
        `  document.head.appendChild(__style);`,
        `}`,
    ].join("\n");
}

async function buildClientBundle() {
    mkdirSync(dirname(OUT_FILE), { recursive: true });

    const result = await esbuild.build({
        entryPoints: [ENTRY_FILE],
        bundle: true,
        write: false,
        format: "cjs",
        platform: "browser",
        target: ["es2022"],
        jsx: "automatic",
        outfile: OUT_FILE,
        external: DSH_CLIENT_EXTERNALS,
        alias: {
            "@": join(REPO_ROOT, "web", "src"),
        },
        define: {
            "process.env.NODE_ENV": '"production"',
        },
        logLevel: "info",
    });

    let jsCode = "";
    let cssCode = "";
    for (const file of result.outputFiles) {
        if (file.path.endsWith(".css")) {
            cssCode += `${file.text}\n`;
        } else if (file.path.endsWith(".js")) {
            jsCode = file.text;
        }
    }

    const styleInjection = buildStyleInjection(cssCode);

    const wrapped = [
        `window.__ModuleLoader__.load({`,
        `  id: "@runlark/dsh-plugin",`,
        `  factory: (require) => {`,
        `    var module = { exports: {} };`,
        `    var exports = module.exports;`,
        styleInjection,
        jsCode,
        `    return module.exports;`,
        `  }`,
        `});`,
        "",
    ].join("\n");

    writeFileSync(OUT_FILE, wrapped, "utf8");
    if (cssCode.trim().length > 0) {
        writeFileSync(OUT_CSS_FILE, cssCode, "utf8");
    }

    const pkg = JSON.parse(
        readFileSync(join(REPO_ROOT, "packages", "dsh-plugin", "package.json"), "utf8"),
    );
    console.log(
        `Built ${pkg.name} DSH client bundle (${Buffer.byteLength(wrapped)} bytes JS, ${Buffer.byteLength(cssCode)} bytes CSS) -> ${OUT_FILE}`,
    );
}

await buildClientBundle();
