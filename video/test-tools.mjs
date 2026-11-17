// Checks for the video tooling. Run: node video/test-tools.mjs  (the Chrome part is skipped
// when Chrome is missing; set CHROME to its path).
import assert from "node:assert/strict";
import { existsSync, mkdtempSync, writeFileSync, rmSync } from "node:fs";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { readEnv } from "./env.mjs";

// .env reader: quotes stripped, env wins, later files only when earlier ones lack the key.
const dir = mkdtempSync(join(tmpdir(), "envtest-"));
writeFileSync(join(dir, "a"), `OTHER=1\nTOK_A="quoted value"\nTOK_B='single'\n`);
writeFileSync(join(dir, "b"), `TOK_C=plain\n`);
assert.equal(readEnv("TOK_A", [join(dir, "a")]), "quoted value");
assert.equal(readEnv("TOK_B", [join(dir, "a")]), "single");
assert.equal(readEnv("TOK_C", [join(dir, "missing"), join(dir, "a"), join(dir, "b")]), "plain");
assert.equal(readEnv("TOK_D", [join(dir, "a")]), undefined);
process.env.TOK_A = "from-env";
assert.equal(readEnv("TOK_A", [join(dir, "a")]), "from-env");
rmSync(dir, { recursive: true });

const chrome = process.env.CHROME || "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
if (!existsSync(chrome)) {
  console.log("video tools: env ok; Chrome not found, launch check skipped");
} else {
  const { launch } = await import("./cdp.mjs");
  // A decoy on the old fixed port must receive nothing.
  let hits = 0;
  const decoy = createServer((s) => { hits++; s.destroy(); });
  await new Promise((r) => decoy.listen(9333, "127.0.0.1", r)).catch(() => {});
  const b = await launch({ width: 200, height: 200 });
  assert.equal(await b.eval("1 + 1"), 2);
  const { profile } = b;
  assert.ok(existsSync(profile));
  await b.close();
  decoy.close();
  assert.ok(!existsSync(profile), "close() removes the Chrome profile");
  assert.equal(hits, 0, "nothing connects to port 9333");
  console.log("video tools: ok");
}
