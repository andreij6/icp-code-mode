#!/usr/bin/env bash
# Fails unless every import of the given Wasm module comes from `ic0` (an ICP replica
# rejects any other import, e.g. wasm-bindgen's __wbindgen_*). Needs wasm-objdump (wabt).
# Usage: scripts/check-wasm-imports.sh path/to/module.wasm
set -euo pipefail
imports=$(wasm-objdump -x -j Import "$1" | grep ' <- ' || true)
[ -n "$imports" ] || { echo "$1: no imports found (wrong file?)" >&2; exit 1; }
bad=$(printf '%s\n' "$imports" | grep -v ' <- ic0\.' || true)
if [ -n "$bad" ]; then
  echo "$1: imports outside ic0:" >&2
  printf '%s\n' "$bad" >&2
  exit 1
fi
echo "$1: $(printf '%s\n' "$imports" | wc -l | tr -d ' ') imports, all from ic0"
