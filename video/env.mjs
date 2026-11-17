// One .env reader for the video tools: the environment wins, then each file in order.
// Surrounding single or double quotes are stripped. Values are never printed.
import { readFileSync } from "node:fs";

export function readEnv(name, files = []) {
  if (process.env[name]) return process.env[name];
  for (const f of files) {
    let text;
    try { text = readFileSync(f, "utf8"); } catch { continue; }
    const m = text.match(new RegExp(`^\\s*(?:export\\s+)?${name}\\s*=(.*)$`, "m"));
    if (m) return m[1].trim().replace(/^(["'])(.*)\1$/, "$2");
  }
}
