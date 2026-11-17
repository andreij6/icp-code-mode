// Render an HTML composition to MP4, frame by frame, deterministically.
// The page must define window.DURATION (seconds) and window.renderAt(t) (sync or async).
// Usage: node video/render.mjs <composition.html> <out.mp4> [audio.wav] [fps]
import { mkdirSync, rmSync, writeFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { resolve } from "node:path";
import { launch } from "./cdp.mjs";

const [comp, out, audio, fpsArg] = process.argv.slice(2);
const fps = Number(fpsArg || 30);
const frames = resolve(out + ".frames");
rmSync(frames, { recursive: true, force: true });
mkdirSync(frames, { recursive: true });

const b = await launch({ width: 1920, height: 1080 });
let duration, n;
try {
  await b.goto("file://" + resolve(comp));
  // Wait for web fonts (window.READY) so no frame renders with fallback type.
  await b.eval("Promise.resolve(window.READY || document.fonts.ready).then(() => true)");
  duration = await b.eval("window.DURATION");
  n = Math.ceil(duration * fps);
  const t0 = Date.now();
  for (let i = 0; i < n; i++) {
    await b.eval(`Promise.resolve(window.renderAt(${(i / fps).toFixed(5)})).then(() => true)`);
    writeFileSync(`${frames}/f${String(i).padStart(5, "0")}.jpg`, await b.shot({ format: "jpeg", quality: 93 }));
    if (i % 150 === 0) process.stderr.write(`frame ${i}/${n} (${((Date.now() - t0) / 1000).toFixed(0)}s)\n`);
  }
} finally {
  await b.close();
}

const args = ["-y", "-loglevel", "error", "-framerate", String(fps), "-i", `${frames}/f%05d.jpg`];
if (audio) args.push("-i", audio);
args.push("-c:v", "libx264", "-preset", "slow", "-crf", "18", "-pix_fmt", "yuv420p", "-movflags", "+faststart");
if (audio) args.push("-c:a", "aac", "-b:a", "192k", "-shortest");
args.push(out);
execFileSync("ffmpeg", args, { stdio: "inherit" });
rmSync(frames, { recursive: true, force: true });
console.log(`wrote ${out}: ${n} frames, ${duration.toFixed(2)}s`);
