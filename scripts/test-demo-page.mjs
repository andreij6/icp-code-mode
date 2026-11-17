// Checks on src/demo.html's own code (no browser). Run: node scripts/test-demo-page.mjs
import { readFileSync } from "node:fs";
import assert from "node:assert/strict";

const html = readFileSync(new URL("../src/demo.html", import.meta.url), "utf8");
// Source of a top-level `const NAME = ...;` or `async function NAME(...) {...}` from the page.
const extract = (start, end) => {
  const i = html.indexOf(start);
  assert.ok(i >= 0, start);
  return html.slice(i, html.indexOf(end, i) + end.length);
};

// 1. A hostile canister id stays a string literal in every scenario script.
const SCENARIOS = new Function(extract("const SCENARIOS = [", "\n];") + "; return SCENARIOS")();
const evil = 'x"; return "pwned"; //';
for (const sc of SCENARIOS) {
  const first = sc.code(evil).split("\n")[0];
  assert.equal(new Function(first + "; return S")(), evil, sc.title);
}

// 2. findStore rejects a canister id that is not principal text.
const findStore = (reply) => new Function("tool", "cache",
  extract("async function findStore()", "\n}\n") + "; return findStore")(async () => ({ text: JSON.stringify(reply) }), {});
const store = (id) => ({ canisters: [{ canister_id: id, allowed_methods: ["*"], methods: [{ name: "list_products", signature: "() -> ()" }] }] });
assert.equal(await findStore(store("aaaaa-aa"))(), "aaaaa-aa");
await assert.rejects(findStore(store(evil))(), /malformed/);

// 3. Classic definitions come from search output, in linear time on a huge hostile reply.
const defs = (reply) => new Function("tool", "cache",
  extract("async function classicDefsChars(S)", "\n}\n") + "; return classicDefsChars")(async () => ({ text: JSON.stringify(reply) }), {});
const S = "aaaaa-aa";
const ok = await defs({ canisters: [{ canister_id: S, methods: [
  { name: "get", signature: "(Id) -> (opt Item) query" },
  { name: "ping", signature: "() -> ()" },
], types: { Id: "nat", Item: "record { id : Id; tags : vec Tag }", Tag: "text" } }] })(S);
assert.equal(ok.count, 2);
const big = { canisters: [{ canister_id: S, methods: [{ name: "m", signature: "(" + "\n".repeat(100_000) + ") -> (" + ") -> (".repeat(20_000) + ")" }], types: {} }] };
const t0 = performance.now();
await defs(big)(S);
assert.ok(performance.now() - t0 < 500, "classicDefsChars is not super-linear");
console.log("demo page: ok");
