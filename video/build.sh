#!/usr/bin/env bash
# Full pipeline: Gemini TTS voiceover -> scene timing -> frame render -> MP4 with audio.
# Needs GEMINI_API_KEY in ../.env. Usage: video/build.sh [voice]
set -euo pipefail
cd "$(dirname "$0")"
node tts.mjs --voice "${1:-Puck}"
Q=$(node -e 'const t=require("./out/timing.json"); console.log(`d=${t.durations.map(d=>d.toFixed(3)).join(",")}&lead=${t.lead}&tail=${t.tail}`)')
node render.mjs "composition.html?$Q" out/icp-code-mode-explainer.mp4 out/voice.wav 30
ffprobe -v error -show_entries format=duration:stream=codec_name,width,height -of compact out/icp-code-mode-explainer.mp4
# The committed copy and README poster (frame from the opening scene).
cp out/icp-code-mode-explainer.mp4 icp-code-mode-explainer.mp4
ffmpeg -loglevel error -y -ss 3.8 -i icp-code-mode-explainer.mp4 -frames:v 1 -vf scale=1280:-1 -q:v 3 poster.jpg
