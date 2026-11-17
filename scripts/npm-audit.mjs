import { spawnSync } from "node:child_process";

// Temporary exemption for upstream dev-only advisory with no published fix yet.
// Auto-fails as soon as `fixAvailable` becomes truthy so we immediately upgrade.
const UNPATCHED_DEV_ADVISORIES = new Map([
    [
        "GHSA-fc5q-39xc-vjpf",
        {
            packageName: "picomatch",
            vulnerableRange: "<=4.0.7",
            reason: "ReDoS in devDependencies (@o-feod/oxlint-structure-plugin, markdownlint-cli2); no picomatch fix <= 4.0.7",
        },
    ],
]);

function runAuditJson(args) {
    const res = spawnSync("npm", args, { encoding: "utf8" });
    if (res.error) {
        throw res.error;
    }
    const raw = (res.stdout || "").trim();
    if (!raw) {
        throw new Error(
            `npm ${args.join(" ")} produced empty output (exit ${res.status}): ${res.stderr}`,
        );
    }
    const report = JSON.parse(raw);
    if (report.error) {
        throw new Error(`npm audit error: ${report.error.summary ?? JSON.stringify(report.error)}`);
    }
    return { status: res.status ?? 0, report };
}

// 1. Strict zero-tolerance check on production dependencies.
const prod = runAuditJson(["audit", "--omit=dev", "--audit-level=low", "--json"]);
const prodTotal = prod.report.metadata?.vulnerabilities?.total ?? 0;
if (prod.status !== 0 || prodTotal > 0) {
    console.error("Production npm audit failed:", JSON.stringify(prod.report.metadata, null, 2));
    process.exit(1);
}

// 2. Full audit across devDependencies: fail on any unexempted advisory or once a fix ships.
const full = runAuditJson(["audit", "--audit-level=low", "--json"]);
const vulns = Object.values(full.report.vulnerabilities ?? {});
const unexpected = [];

for (const entry of vulns) {
    const viaObjects = (entry.via ?? []).filter((v) => typeof v === "object" && v !== null);
    for (const adv of viaObjects) {
        const ghsaMatch = /\/advisories\/(GHSA-[a-z0-9-]+)/iu.exec(adv.url ?? "");
        const ghsa = ghsaMatch ? ghsaMatch[1] : String(adv.source ?? adv.title ?? "unknown");
        const allowed = UNPATCHED_DEV_ADVISORIES.get(ghsa);
        const directPkgFix =
            typeof entry.fixAvailable === "object" &&
            entry.fixAvailable !== null &&
            entry.fixAvailable.name === allowed?.packageName;
        const rangeMismatch =
            allowed?.vulnerableRange !== undefined && adv.range !== allowed.vulnerableRange;
        if (!allowed || allowed.packageName !== adv.name || directPkgFix || rangeMismatch) {
            unexpected.push({
                ghsa,
                package: adv.name ?? entry.name,
                severity: adv.severity ?? entry.severity,
                fixAvailable: entry.fixAvailable,
                url: adv.url,
            });
        }
    }
}

if (unexpected.length > 0 || (full.status !== 0 && vulns.length === 0)) {
    console.error("npm audit failed on unexpected or fixable advisories:", unexpected);
    process.exit(1);
}

if (vulns.length > 0) {
    console.log(
        `npm audit passed (0 prod vulnerabilities; ${vulns.length} transitive dev nodes tied strictly to unpatched GHSA-fc5q-39xc-vjpf).`,
    );
} else {
    console.log("npm audit passed (0 vulnerabilities).");
}
