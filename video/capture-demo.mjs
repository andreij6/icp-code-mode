// Screenshots of the live demo page (real canister, real calls) for the video.
// Usage: node video/capture-demo.mjs   (reads MCP_TOKEN from the env or ../.env; never prints it)
import { writeFileSync } from "node:fs";
import { launch } from "./cdp.mjs";
import { readEnv } from "./env.mjs";

const here = new URL(".", import.meta.url).pathname;
const token = readEnv("MCP_TOKEN", [here + "../.env"]);
if (!token) throw new Error("MCP_TOKEN missing: set it in the env or .env");
const URL_ = process.env.DEMO_URL || "http://icp_code_mode.local.localhost:8010/";
const b = await launch({ width: 1100, height: 1400 });
try {
  await b.send("Emulation.setDeviceMetricsOverride", { width: 1100, height: 1400, deviceScaleFactor: 2, mobile: false });
  // Light theme, to match the video's dashboard-style design (headless Chrome follows the OS otherwise).
  await b.send("Emulation.setEmulatedMedia", { features: [{ name: "prefers-color-scheme", value: process.env.SCHEME || "light" }] });
  await b.goto(URL_);
  await b.eval(`sessionStorage.setItem("icp-code-mode-token", ${JSON.stringify(token)}); location.reload(); true`);
  await new Promise((r) => setTimeout(r, 1500));
  // Document-coordinate box (CSS px) for a clip; captureBeyondViewport handles offscreen parts.
  const box = (sel, pad = 12) => b.eval(`(() => { const r = document.querySelector(${JSON.stringify(sel)}).getBoundingClientRect();
    return { x: r.left + scrollX - ${pad}, y: r.top + scrollY - ${pad}, width: r.width + ${2 * pad}, height: r.height + ${2 * pad} }; })()`);
  const shot = async (sel, file) => {
    const clip = await box(sel);
    const r = await b.send("Page.captureScreenshot", { format: "png", captureBeyondViewport: true, clip: { ...clip, scale: 1 } });
    writeFileSync(here + "assets/" + file, Buffer.from(r.data, "base64"));
  };
  const out = {};
  for (const [i, name] of [[0, "top3"], [1, "restock"], [2, "tea"]]) {
    await b.eval(`(async () => { document.querySelectorAll('#scenarios button')[${i}].click(); document.getElementById('btn-compare').click();
      await new Promise(r => setTimeout(r, 300));
      for (let k = 0; k < 200 && document.getElementById('btn-compare').disabled; k++) await new Promise(r => setTimeout(r, 100));
      return document.getElementById('cmp-status').textContent; })()`);
    await shot("#cmp-out .cols", `${name}-columns.png`);
    await shot("#cmp-table", `${name}-table.png`);
    out[name] = await b.eval(`({ headline: document.getElementById('cmp-headline').textContent, same: document.getElementById('cmp-same').textContent,
      rows: [...document.querySelectorAll('#cmp-table tr')].slice(1).map(r => [...r.children].map(c => c.textContent)),
      classic: [...document.querySelectorAll('#classic-turns li')].map(l => l.textContent),
      code: [...document.querySelectorAll('#code-turns li')].map(l => l.textContent) })`);
  }
  // The whole comparison section as one "this is the real app" shot.
  await b.eval(`document.querySelectorAll('#scenarios button')[0].click(); document.getElementById('btn-compare').click(); true`);
  await new Promise((r) => setTimeout(r, 2500));
  await shot("section:nth-of-type(2)", "page-comparison.png");
  writeFileSync(here + "assets/demo-results.json", JSON.stringify(out, null, 2));
  console.log(Object.entries(out).map(([k, v]) => `${k}: ${v.headline} | same ${v.same}`).join("\n"));
} finally {
  await b.close();
}
