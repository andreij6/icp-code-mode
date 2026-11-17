#!/usr/bin/env bash
# Runs scripts/demo-setup.sh against a stub `icp` that records its arguments.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
cat > "$tmp/icp" <<'STUB'
#!/usr/bin/env bash
echo "$*" >> "$ICP_STUB_LOG"
case " $* " in *" --id-only "*) echo aaaaa-aa ;; *" status "*) echo "Canister Id: wrong-id" ;; esac
STUB
chmod +x "$tmp/icp"
export ICP_STUB_LOG="$tmp/log"
fail() { echo "FAIL: $1"; cat "$ICP_STUB_LOG"; exit 1; }

PATH="$tmp:$PATH" ICP_ENVIRONMENT=audit "$here/demo-setup.sh" >/dev/null
[ "$(grep -c -- '-e audit' "$ICP_STUB_LOG")" = 2 ] || fail "ICP_ENVIRONMENT not used by both calls"
grep -q 'status demo_store .*--id-only' "$ICP_STUB_LOG" || fail "status not called with --id-only"
grep -q 'principal "aaaaa-aa"' "$ICP_STUB_LOG" || fail "id from --id-only not used"

: > "$ICP_STUB_LOG"
PATH="$tmp:$PATH" ICP_ENVIRONMENT=audit "$here/demo-setup.sh" ic >/dev/null
[ "$(grep -c -- '-e ic' "$ICP_STUB_LOG")" = 2 ] || fail "explicit argument not used"
echo "demo-setup.sh: ok"
