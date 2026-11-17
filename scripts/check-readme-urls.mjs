// Fails if a local MCP endpoint URL in the README uses a `*.localhost` subdomain (such as
// icp_code_mode.local.localhost). curl and browsers resolve those, but Node and Python MCP
// clients on macOS do not; client-facing URLs must use localhost:<port>/mcp?canisterId=<id>.
// A static rule, not a DNS lookup: glibc resolves *.localhost, so a lookup on Linux CI
// would pass the very URL that fails on a Mac. Browser-only URLs (no /mcp) are not checked.
// Usage: node scripts/check-readme-urls.mjs [README.md]
import { readFileSync } from "node:fs";

const file = process.argv[2] || new URL("../README.md", import.meta.url);
const bad = [];
readFileSync(file, "utf8").split("\n").forEach((line, i) => {
  for (const [url] of line.matchAll(/https?:\/\/[^\s"'`)<>]+/g)) {
    const u = new URL(url);
    if (u.pathname.replace(/\/$/, "").endsWith("/mcp") && u.hostname.endsWith(".localhost")) bad.push(`${i + 1}: ${url}`);
  }
});
if (bad.length) {
  console.error(`MCP URLs a Node or Python client cannot resolve (use http://localhost:<port>/mcp?canisterId=<id>):\n${bad.join("\n")}`);
  process.exit(1);
}
console.log("README MCP URLs: ok");
