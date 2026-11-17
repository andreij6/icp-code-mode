#!/usr/bin/env bash
# Allowlist the demo store in icp_code_mode: all 7 reads plus create_order.
# cancel_order and restock stay blocked for scripts (icp_code_mode's allowlist) so the
# demo can show the guardrail; the store itself accepts them from anyone (local demo only).
# Usage: scripts/demo-setup.sh [environment]   (default: local)
set -euo pipefail
ENV="${1:-local}"
STORE=$(icp canister status demo_store -e "$ENV" | sed -n 's/^Canister Id: *//p')
[ -n "$STORE" ] || { echo "demo_store is not deployed in '$ENV' (run: icp deploy)" >&2; exit 1; }
icp canister call icp_code_mode -e "$ENV" set_allowlist "(principal \"$STORE\", vec {
  \"list_products\"; \"get_product\"; \"search_products\"; \"get_inventory\";
  \"get_customer\"; \"list_orders\"; \"sales_summary\"; \"create_order\" })" >/dev/null
echo "Allowlisted demo_store ($STORE) in icp_code_mode [$ENV]."
