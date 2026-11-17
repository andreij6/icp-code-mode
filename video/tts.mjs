// Voiceover with Gemini TTS: one clip per scene, so the video can be timed to the real audio.
// GEMINI_API_KEY comes from the env, this project's .env, or ~/.env (the same convention as
// leet-clone's tools/video/tts); it is never printed. Writes out/voice.wav and out/timing.json.
// Usage: node video/tts.mjs [--voice Puck] [--model <name>] [--list]
import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { homedir } from "node:os";

const here = new URL(".", import.meta.url).pathname;
const arg = (k, d) => { const i = process.argv.indexOf(k); return i > 0 ? process.argv[i + 1] : d; };
const fromFile = (f) => { try { return (readFileSync(f, "utf8").match(/^GEMINI_API_KEY=(.+)$/m) || [])[1]?.trim().replace(/^["']|["']$/g, ""); } catch {} };
const KEY = process.env.GEMINI_API_KEY || fromFile(here + "../.env") || fromFile(homedir() + "/.env");
if (!KEY) { console.error("GEMINI_API_KEY missing: set it in the env, .env or ~/.env"); process.exit(1); }
const API = "https://generativelanguage.googleapis.com/v1beta";
const headers = { "x-goog-api-key": KEY, "content-type": "application/json" };

async function ttsModels() {
  const names = [];
  let page = "";
  do {
    const r = await (await fetch(`${API}/models?pageSize=200${page ? "&pageToken=" + page : ""}`, { headers })).json();
    if (r.error) throw new Error(`models: ${r.error.status} ${r.error.message}`);
    for (const m of r.models || []) if (/tts/i.test(m.name)) names.push(m.name.replace(/^models\//, ""));
    page = r.nextPageToken;
  } while (page);
  // Newest version first; then GA over preview, full over lite.
  const score = (n) => parseFloat((n.match(/(\d+(?:\.\d+)?)/) || [0, 0])[1]) * 100 + (/preview/.test(n) ? 0 : 10) + (/lite/.test(n) ? 0 : 5);
  return names.sort((a, b) => score(b) - score(a));
}

const models = await ttsModels();
if (process.argv.includes("--list")) { console.log(models.join("\n")); process.exit(0); }
const MODEL = arg("--model", models[0]);
const VOICE = arg("--voice", "Puck");
if (!MODEL) throw new Error("no TTS model available to this key");
console.log(`model ${MODEL}, voice ${VOICE}`);

const script = JSON.parse(readFileSync(here + "script.json", "utf8"));
// gemini-3.x TTS reads any instruction prefix aloud and rejects system instructions, so only
// the line itself is sent; tone comes from the voice (Puck: upbeat). Older 2.5 models do take a
// style prefix, so it is used there.
const STYLE = /2\.5/.test(MODEL) ? "Say in a confident, upbeat, crisp developer-launch tone" : "";
const GAP = 0.35, LEAD = 0.3, TAIL = 1.2;
mkdirSync(here + "out", { recursive: true });

async function speak(text) {
  const body = {
    contents: [{ parts: [{ text: STYLE ? `${STYLE}: ${text}` : text }] }],
    generationConfig: { responseModalities: ["AUDIO"], speechConfig: { voiceConfig: { prebuiltVoiceConfig: { voiceName: VOICE } } } },
  };
  for (let attempt = 1; ; attempt++) {
    const res = await fetch(`${API}/models/${MODEL}:generateContent`, { method: "POST", headers, body: JSON.stringify(body) });
    const r = await res.json();
    const part = r.candidates?.[0]?.content?.parts?.find((p) => p.inlineData);
    if (part) return { pcm: Buffer.from(part.inlineData.data, "base64"), mime: part.inlineData.mimeType };
    if (attempt >= 4) throw new Error(`TTS failed: ${JSON.stringify(r.error || r).slice(0, 300)}`);
    await new Promise((s) => setTimeout(s, 1500 * attempt));
  }
}

const ff = (...a) => execFileSync("ffmpeg", ["-y", "-loglevel", "error", ...a]);
const dur = (f) => Number(execFileSync("ffprobe", ["-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0", f]).toString());
const clips = [];
for (const [i, s] of script.scenes.entries()) {
  const { pcm, mime } = await speak(s.voiceover);
  // Newer models return a WAV file; older ones raw 16-bit PCM ("audio/L16;rate=24000").
  const isWav = /wav/i.test(mime);
  const rate = Number((mime.match(/rate=(\d+)/) || [0, 24000])[1]);
  const raw = `${here}out/scene${i + 1}.${isWav ? "src.wav" : "pcm"}`, wav = `${here}out/scene${i + 1}.wav`;
  writeFileSync(raw, pcm);
  const input = isWav ? ["-i", raw] : ["-f", "s16le", "-ar", String(rate), "-ac", "1", "-i", raw];
  // Trim leading/trailing silence, then add the inter-scene gap.
  ff(...input, "-af",
    `silenceremove=start_periods=1:start_threshold=-45dB,areverse,silenceremove=start_periods=1:start_threshold=-45dB,areverse,apad=pad_dur=${GAP}`,
    "-ar", "48000", wav);
  clips.push(wav);
  console.log(`scene ${i + 1}: ${dur(wav).toFixed(2)}s`);
}
const durations = clips.map(dur);
const list = `${here}out/concat.txt`;
writeFileSync(list, clips.map((c) => `file '${c}'`).join("\n"));
ff("-f", "concat", "-safe", "0", "-i", list, "-af",
  `adelay=${LEAD * 1000}|${LEAD * 1000},apad=pad_dur=${TAIL},loudnorm=I=-16:TP=-1.5:LRA=11`, "-ar", "48000", "-ac", "2", `${here}out/voice.wav`);
writeFileSync(`${here}out/timing.json`, JSON.stringify({ model: MODEL, voice: VOICE, lead: LEAD, tail: TAIL, durations }, null, 2));
console.log(`voice.wav ${dur(here + "out/voice.wav").toFixed(2)}s; durations ${durations.map((d) => d.toFixed(2)).join(",")}`);
