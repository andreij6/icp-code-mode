// Minimal Chrome DevTools Protocol driver (no dependencies; Node >= 22 for WebSocket).
import { spawn } from "node:child_process";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const CHROME = process.env.CHROME || "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";

export async function launch({ width = 1920, height = 1080, port = 9333 } = {}) {
  const profile = mkdtempSync(join(tmpdir(), "cdp-"));
  const proc = spawn(CHROME, [
    "--headless=new", `--remote-debugging-port=${port}`, `--user-data-dir=${profile}`,
    "--hide-scrollbars", "--mute-audio", "--no-first-run", "--no-default-browser-check",
    "--force-color-profile=srgb", "--font-render-hinting=none", `--window-size=${width},${height}`, "about:blank",
  ], { stdio: "ignore" });
  let targets;
  for (let i = 0; i < 100; i++) {
    try { targets = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json(); if (targets.length) break; } catch {}
    await new Promise((r) => setTimeout(r, 100));
  }
  const page = targets.find((t) => t.type === "page");
  const ws = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((r, j) => { ws.onopen = r; ws.onerror = j; });
  let id = 0;
  const pending = new Map(), listeners = [];
  ws.onmessage = (m) => {
    const msg = JSON.parse(m.data);
    if (msg.id && pending.has(msg.id)) {
      const { resolve, reject } = pending.get(msg.id); pending.delete(msg.id);
      msg.error ? reject(new Error(JSON.stringify(msg.error))) : resolve(msg.result);
    } else if (msg.method) listeners.forEach((l) => l(msg));
  };
  const send = (method, params = {}) => new Promise((resolve, reject) => {
    const i = ++id; pending.set(i, { resolve, reject }); ws.send(JSON.stringify({ id: i, method, params }));
  });
  const once = (method) => new Promise((r) => { const l = (m) => { if (m.method === method) { listeners.splice(listeners.indexOf(l), 1); r(m.params); } }; listeners.push(l); });
  await send("Page.enable");
  await send("Emulation.setDeviceMetricsOverride", { width, height, deviceScaleFactor: 1, mobile: false });
  const api = {
    send,
    async goto(url) { const loaded = once("Page.loadEventFired"); await send("Page.navigate", { url }); await loaded; },
    async eval(expression) {
      const r = await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
      if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description || r.exceptionDetails.text);
      return r.result.value;
    },
    async shot({ format = "jpeg", quality = 92, clip } = {}) {
      const r = await send("Page.captureScreenshot", { format, quality, ...(clip ? { clip: { ...clip, scale: clip.scale || 1 } } : {}) });
      return Buffer.from(r.data, "base64");
    },
    async close() { try { ws.close(); } catch {} proc.kill(); },
  };
  return api;
}
