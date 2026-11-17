// Minimal Chrome DevTools Protocol driver (no dependencies; Node >= 22 for WebSocket).
import { spawn } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const CHROME = process.env.CHROME || "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";

// Chrome picks a free debugging port (0) and writes it to <profile>/DevToolsActivePort, so
// nothing else listening on a fixed port can receive our commands (or the page's token).
// The profile can hold page storage (the demo token): close() deletes it, and exit and
// SIGINT/SIGTERM hooks do the same if the caller dies first.
export async function launch({ width = 1920, height = 1080 } = {}) {
  const profile = mkdtempSync(join(tmpdir(), "cdp-"));
  const proc = spawn(CHROME, [
    "--headless=new", "--remote-debugging-port=0", `--user-data-dir=${profile}`,
    "--hide-scrollbars", "--mute-audio", "--no-first-run", "--no-default-browser-check",
    "--force-color-profile=srgb", "--font-render-hinting=none", `--window-size=${width},${height}`, "about:blank",
  ], { stdio: "ignore" });
  const exited = new Promise((r) => proc.once("exit", r));
  const cleanup = () => { proc.kill("SIGKILL"); rmSync(profile, { recursive: true, force: true, maxRetries: 5 }); };
  const onSignal = (sig) => { cleanup(); process.exit(sig === "SIGINT" ? 130 : 143); };
  const unhook = () => { process.off("exit", cleanup); process.off("SIGINT", onSignal); process.off("SIGTERM", onSignal); };
  process.once("exit", cleanup);
  process.once("SIGINT", onSignal);
  process.once("SIGTERM", onSignal);
  let ws;
  try {
    let port, targets;
    for (let i = 0; i < 100 && !targets?.length; i++) {
      await new Promise((r) => setTimeout(r, 100));
      try {
        port ||= Number(readFileSync(join(profile, "DevToolsActivePort"), "utf8").split("\n")[0]) || undefined;
        if (port) targets = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
      } catch {}
    }
    const page = targets?.find((t) => t.type === "page");
    if (!page) throw new Error("Chrome did not start (set CHROME to its path)");
    ws = new WebSocket(page.webSocketDebuggerUrl);
    await new Promise((r, j) => { ws.onopen = r; ws.onerror = j; });
  } catch (e) {
    cleanup(); unhook(); throw e;
  }
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
    profile,
    async close() {
      try { ws.close(); } catch {}
      proc.kill();
      await Promise.race([exited, new Promise((r) => setTimeout(r, 3000))]);
      cleanup(); unhook();
    },
  };
  return api;
}
