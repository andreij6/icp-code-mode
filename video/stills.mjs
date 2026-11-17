// Capture still frames of the composition at given times: node video/stills.mjs "<query>" t1 t2 ...
import { writeFileSync, mkdirSync } from "node:fs";
import { resolve } from "node:path";
import { launch } from "./cdp.mjs";
const [query, ...times] = process.argv.slice(2);
const dir = resolve(new URL(".", import.meta.url).pathname, "stills");
mkdirSync(dir, { recursive: true });
const b = await launch();
await b.goto("file://" + resolve(new URL(".", import.meta.url).pathname, "composition.html") + (query ? "?" + query : ""));
// Wait for web fonts (window.READY) so no frame renders with fallback type.
await b.eval("Promise.resolve(window.READY || document.fonts.ready).then(() => true)");
console.log("duration", await b.eval("window.DURATION"));
for (const t of times) {
  await b.eval(`Promise.resolve(window.renderAt(${t})).then(() => true)`);
  writeFileSync(`${dir}/t${t}.jpg`, await b.shot({ quality: 85 }));
}
await b.close();
