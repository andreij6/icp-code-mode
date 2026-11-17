# Per-Customer Canisters — Architecture & Implementation Plan

> **Status:** Planning & architecture specification (no code changes implemented).
> **Companion doc:** [per-customer-canisters.md](./per-customer-canisters.md)

## 1. Review of Current Readiness in `icp_code_mode`

The existing canister in [src/lib.rs](../src/lib.rs) and [icp_code_mode.did](../icp_code_mode.did) already provides almost everything needed for single-tenant customer instances:

- **Provisioning via `InitArgs`:** Accepts `token_sha256: vec text`, `allowlist: vec record { principal; vec text }`, and `limits: opt Limits`.
- **Safe upgrades via stable memory:** `LIMITS`, `ALLOWLIST`, `TOKENS`, `AUDIT`, and `CANDID_OVERRIDES` live in `ic-stable-structures` ([src/policy.rs](../src/policy.rs)), so upgrades preserve customer state without `pre_upgrade` serialization.
- **Controller-scoped admin surface:** `set_allowlist`, `remove_allowlist`, `set_limits`, `add_api_token_hash`, `revoke_api_token_hash`, `set_candid_override`, `get_policy`, `get_audit_logs`, and `get_audit_log_len` all enforce `ic_cdk::api::is_controller`.
- **Runtime feature flags:** [Cargo.toml](../Cargo.toml) gates `js`, `python`, and `rhai` behind Cargo features, letting the factory offer smaller Wasm builds (~6 MB for JS+Rhai vs. ~12 MB with Python) with roughly half the storage cycle cost.

### Gaps & Constraints Identified in the Existing Codebase

1. **Wasm size exceeds the 2 MiB message limit:** The all-in-one Wasm is ~12 MB (~3.6 MB gzipped), so a factory cannot call single-message `install_code`. It must use the IC management canister's chunked install API (`upload_chunk` + `install_chunked_code`).
2. **`InitArgs` validation traps in `init`:** `apply()` in `src/lib.rs` calls `ic_cdk::trap` if any `token_sha256` is not 64 hex chars. If the factory creates a canister first (~0.5T cycles spent) and `init` traps, cycles are wasted on an uninitialized canister. The factory must pre-validate `InitArgs` before calling `create_canister`.
3. **Unbounded call waits can block upgrades:** `Limits.call_timeout_seconds` defaults to `None` (`Call::unbounded_wait`). A customer calling a stalled target canister could block `stop_canister` during an upgrade unless new instances default to a bounded timeout (e.g., `Some(30)`).
4. **`post_upgrade` ignores its arguments (resolved):** `InitArgs` apply at install only, so an upgrade cannot bring back revoked tokens or overwrite `allowlist`/`limits`, whatever it is given. Token, allowlist and limit changes after install go through the controller endpoints.
5. **Single-canister allowlist mutations post-init:** `InitArgs.allowlist` already accepts a `vec` of canisters at creation time, but after initialization `icp_code_mode` only exposes single-canister `set_allowlist` and `remove_allowlist` endpoints. Adding batch endpoints (`set_allowlists` / `remove_allowlists` / `replace_allowlist`) avoids N sequential controller calls when managing lists of canisters.
6. **Customer instance demo UI & teardown:** `http_request` currently serves `src/demo.html` (hardcoded `demo_store` benchmarks) whenever `certified_reads` is off, and `icp_code_mode` has no controller-only `drain_cycles` endpoint to reclaim remaining cycles before `delete_canister`.
7. **No factory canister, payment flow, or release registry exists yet.**

---

## 2. Target Architecture

```
                         ┌──────────────────────────────┐
                         │       Customer / Wallet      │
                         └──────────────┬───────────────┘
                                        │
           1. Approve / pay cycles/ICP  │  2. MCP client (HTTPS /mcp + Bearer)
                                        ▼
┌───────────────────────────────────────┴───────────────────────────────────────┐
│                         Factory Canister (icp_code_mode_factory)              │
│  - Release Wasm store (chunked blobs by variant + SHA-256)                    │
│  - Instance registry (owner -> instances, state machine, version)             │
│  - Payment accounting (cycles or ICP/ICRC-2 -> CMC conversion)                │
│  - Web UI / onboarding page (http_request)                                    │
└──────────────────────────┬─────────────────────────────────┬──────────────────┘
                           │ upload_chunk /                  │
                           │ install_chunked_code            │ (optional co-controller
                           ▼                                 │  status / upgrade)
              ┌─────────────────────────┐                    │
              │  Customer Instance #1   │◀───────────────────┘
              │  (icp_code_mode Wasm)   │
              │  - Own Principal ID     │────▶ Target Canisters (allowlisted)
              │  - Own Allowlist/Tokens │
              │  - Own StableLog Audit  │
              │  - Own Cycle Balance    │
              └─────────────────────────┘
```

### Multi-Variant Wasm Catalog

The factory can distribute up to three pre-built release flavors (matching [Cargo.toml](../Cargo.toml) features):

| Variant | Included Engines | Approx. Wasm Size | Annual Idle Cost (est.) | Best For |
|---|---|---|---|---|
| `full` | JS + Python + Rhai | ~12 MB (~3.6 MB gz) | ~$10 / yr | General LLM agents (default) |
| `js-rhai` | JS + Rhai | ~6 MB | ~$5 / yr | Cost-sensitive / JS-first workloads |
| `rhai` | Rhai only | Smallest | Lowest | Deterministic rule/automation scripts |

---

## 3. Factory Canister Specification

### 3.1 State Model (Stable Memory)

- **`RELEASES` (`StableBTreeMap<(Variant, Version), ReleaseMeta>`):** Stores Wasm SHA-256, chunk hashes, upload state (`Uploading` | `Ready` | `Deprecated`), whether its chunks are staged in the factory's IC chunk store, and total byte size.
- **`CHUNKS` (`StableBTreeMap<[u8; 32], Vec<u8>>`):** Content-addressed Wasm chunks (≤ 1 MB each, safely below the 2 MiB IC message limit).
- **`INSTANCES` (`StableBTreeMap<Principal, InstanceRecord>`):**
  - `owner: Principal`
  - `canister_id: Option<Principal>`
  - `subnet_id: Option<Principal>` (tracks same-subnet vs. cross-subnet placement)
  - `variant: Variant`
  - `version: String`
  - `status: ProvisionStatus` (`Reserved`, `Created`, `Installing`, `Installed`, `Deleting`, `Failed { step, reason }`)
  - `controller_mode: ControllerMode` (`CustomerOnly`, `CoControlled`)
  - `created_at_ns: u64`
- **`BY_OWNER` (`StableBTreeMap<(Principal, Principal), ()>`):** Secondary index for fast `my_instances()` lookup.
- **`CREDITS` / `DEPOSITS` (`StableBTreeMap<Principal, u128>`):** Refundable balance ledger so failed provisions never lose customer funds.
- **`CONFIG` (`StableCell<FactoryConfig>`):** Setup fee, token-conversion margin bps (0% on raw cycle pass-through), minimum initial cycles, max concurrent in-flight installs, default `Limits` for new instances, treasury principal, and pausing flag.

### 3.2 Chunked Release Publishing & Provisioning State Machine

#### Admin Release Upload (respects the 2 MiB IC message limit)
1. `begin_release(variant, version, total_bytes, wasm_sha256) -> Result<ReleaseId, String>`
2. `upload_release_chunk(release_id, chunk_index, bytes) -> Result<(), String>` (≤ 1 MB per message; stores chunks in `CHUNKS` keyed by SHA-256)
3. `finalize_release(release_id) -> Result<(), String>` — verifies full Wasm size and SHA-256, then calls `mgmt::upload_chunk { canister_id: factory, chunk }` for each chunk of active releases so the factory's own chunk store holds one shared copy (subject to the IC's 100-chunk / 100 MiB chunk-store cap; older releases stay in `CHUNKS` and can be re-staged on demand).

#### Customer Provisioning (`create_instance`) & Failure Recovery
Because provisioning spans multiple inter-canister `await`s, the factory treats creation as an **idempotent, locked state machine** per `request_id`:

1. **Validate inputs synchronously (before spending cycles):**
   - Check in-flight `Installing` / `Deleting` count is below `max_concurrent_installs`.
   - Verify `token_sha256` entries are 64-char hex strings.
   - Verify any initial `allowlist` entries (no duplicate `Principal`s, non-empty method lists, capped entry count so `InitArgs` stays well under message limits).
   - Verify requested `variant` has a `Ready` release.
   - Verify payment/deposit covers `creation_fee + setup_fee + min_initial_cycles`.
2. **Reserve funds & record `ProvisionStatus::Reserved`:**
   - Debit customer balance (or accept attached cycles).
3. **Create canister (`mgmt::create_canister`):**
   - Attach `deposit - setup_fee` cycles.
   - Set initial controllers to `[factory]` temporarily (so the factory can install without race conditions), plus recommended `freezing_threshold` (e.g., 30 days).
   - Persist `canister_id` and transition to `ProvisionStatus::Created`.
4. **Subnet-aware chunked install (lock with `ProvisionStatus::Installing` before awaiting):**
   - **Same-subnet fast path (default for `mgmt::create_canister`):** Call `mgmt::install_chunked_code { target_canister: target, store_canister: Some(factory), ... }` directly from the factory's pre-staged chunk store—zero per-customer chunk uploads.
   - **Cross-subnet fallback (when using CMC subnet placement):** Upload chunks to `target` via `mgmt::upload_chunk`, install with `store_canister: Some(target)`, and call `mgmt::clear_chunk_store`.
   - Pass `arg = Encode!(&Some(init_args))`.
5. **Finalize controllers (`mgmt::update_settings`):**
   - `CustomerOnly` (**default**): set `controllers = [customer]`.
   - `CoControlled` (explicit opt-in): set `controllers = [customer, factory]`.
   - Transition to `ProvisionStatus::Installed`.
6. **Recovery, Retry & Cycle Reclaim:**
   - If step 3 fails, refund the customer's credit balance immediately.
   - If step 4 or 5 fails after `create_canister` succeeded, transition to `ProvisionStatus::Failed { step, reason }` with `canister_id` recorded.
   - `retry_install(canister_id)` requires `caller == owner` and atomically locks `Created | Failed` → `Installing` before awaiting, preventing concurrent install races.
   - `delete_failed_instance(canister_id)` requires `caller == owner` and `status` in `Created | Failed` (while the factory still holds temporary control): transitions to `Deleting`, installs a tiny single-message cycle-drain stub (via `install_code`) if needed to send remaining cycles back to `CREDITS[owner]` (`mgmt::delete_canister` burns any leftover cycles if not drained first), then calls `mgmt::stop_canister` + `mgmt::delete_canister`.

### 3.3 Factory Public API (Candid)

- **Customer endpoints:**
  - `get_catalog() -> (FactoryPricing, vec ReleaseInfo)` (query; includes curated allowlist presets)
  - `create_instance(CreateArgs) -> Result<Principal, CreateError>` (update; accepts `token_sha256`, initial `allowlist`, `controller_mode` defaulting to `CustomerOnly`, and optional `limits`)
  - `retry_install(Principal) -> Result<(), String>` (update; owner-only, locked via `Installing`)
  - `delete_failed_instance(Principal) -> Result<nat, String>` (update; owner-only, drains and refunds remaining cycles to `CREDITS` before `delete_canister`)
  - `withdraw_credits(nat) -> Result<(), String>` (update; withdraws refunded credits)
  - `top_up(Principal) -> Result<nat, String>` (update; 0% margin on raw cycle pass-through, or converts ICP/ckUSDC in Phase 2 minus conversion fee)
  - `my_instances() -> vec InstanceRecord` (query)
  - `get_instance_health(Principal) -> Result<InstanceHealth, String>` (update, for `CoControlled` instances)
  - `upgrade_instance(Principal, UpgradeArgs) -> Result<(), String>` (update, for `CoControlled` instances; checks `canister_status` first and syncs if factory was removed externally)
  - `renounce_factory_control(Principal) -> Result<(), String>` (update; fetches live `canister_status().settings.controllers`, removes only `factory`, preserves any other customer controllers, or syncs `INSTANCES` if already removed)
- **Admin (factory controller-only) endpoints:**
  - `begin_release(Variant, Version, nat64, text) -> Result<ReleaseId, String>`
  - `upload_release_chunk(ReleaseId, nat32, blob) -> Result<(), String>`
  - `finalize_release(ReleaseId) -> Result<(), String>`
  - `deprecate_release(Variant, Version) -> Result<(), String>`
  - `set_factory_config(FactoryConfig)`
  - `withdraw_fees(Principal, nat)`
  - `set_paused(bool)`

---

## 4. Self-Serve Allowlist Management

Customers need to manage **lists of target canisters** both when spinning up an instance and throughout its lifecycle—without needing manual `icp canister call` commands one canister at a time.

### 4.1 Provision-Time Bulk Allowlist (`create_instance`)

- `CreateArgs` accepts `allowlist: vec record { principal; vec text }`, forwarded directly into `icp_code_mode`'s existing `InitArgs.allowlist`.
- The onboarding UI lets customers build this initial list from:
  - **Paste / import:** A list of canister IDs (one per line, CSV, or JSON) with a default method policy (explicit method list or `["*"]`).
  - **Curated presets:** One-click packs for common ICP canisters (e.g., ICP Ledger, ckBTC, ckETH, NNS Governance) pre-configured with **read-only** methods instead of `["*"]`.

### 4.2 Post-Creation Bulk Allowlist API (`icp_code_mode`)

To let customers update dozens of canisters in a single atomic transaction after deployment, add controller-only batch methods and cycle-reclaim support to `icp_code_mode`:

- `set_allowlists : (vec record { principal; vec text }) -> (Result)` — validates non-empty method lists (rejecting `vec {}` with an error telling callers to use `remove_allowlists`), rejects duplicate principals, caps batch size (e.g., ≤ 200 entries), upserts all entries atomically, and evicts updated principals from `DID_CACHE`.
- `remove_allowlists : (vec principal) -> (nat32)` — removes a list of canisters and evicts any cached `.did` entries from `DID_CACHE`, returning how many were removed.
- `replace_allowlist : (vec record { principal; vec text }) -> (Result)` — runs the same batch validation and atomically swaps the entire allowlist (useful for declarative JSON configs), clearing `DID_CACHE` for removed/changed entries.
- `drain_cycles : (principal) -> (nat)` — controller-only helper that sends all cycles above a minimal reserve back to a customer-specified recipient so instances can be cleanly decommissioned before `delete_canister`.

### 4.3 Self-Serve UI, Identity Consistency & Candid Overrides

- **Fully certified factory UI (`ic-http-certification`):** Because the factory UI handles payments, `token_sha256` generation, and controller principals, it must serve all HTML/JS/CSS with full `ic-http-certification` asset trees—never `SKIP_CERT_DATA`—so a malicious boundary/replica node cannot tamper with client code.
- **Identity & origin consistency (II `derivationOrigin` trap):** Internet Identity derives a different principal per origin (`<factory-id>.icp0.io` vs. `<instance-id>.icp0.io`). To prevent controller-principal mismatches:
  - Keep all instance management actions (`get_policy`, `set_allowlists`, `remove_allowlists`, `set_candid_override`, `add_api_token_hash`, `get_audit_logs`) inside the **factory's certified origin** (`https://<factory-id>.icp0.io`), calling `<instance-id>` directly from the browser via `@dfinity/agent` with the customer's factory-origin principal `P_factory`.
  - Optionally support wallet adapters (Plug, Oisy) or configure explicit II `derivationOrigin` if any secondary UI runs on another domain.
- **Privilege separation:** Keeping allowlist mutations strictly restricted to controller principals guarantees that an LLM, MCP client, or leaked Bearer token can **never** escalate its own permissions or add new canisters to the allowlist.
- **Method picker, `.did` overrides & guardrails in UI:**
  - Validate each pasted canister ID client-side and pre-fetch/preview its `.did` methods when public `candid:service` metadata or `__get_candid_interface_tmp_hack` is available.
  - When a target canister does not expose its interface, let the customer paste or upload a `.did` file directly in the UI, which calls `set_candid_override(canister, Some(did))` on their instance.
  - Display a clear warning banner whenever `["*"]` (all methods, including state-changing calls) is chosen for a canister.
  - Show a diff preview (`+ add`, `~ update`, `- remove`) against `get_policy().allowlist` before signing.

---

## 5. Key Architectural Decisions & Trade-Offs

### A. Payment Model & Unit Economics
| Option | UX | Complexity | Recommended Phase |
|---|---|---|---|
| **Direct Cycles (`msg_cycles_accept`)** | Requires cycles wallet / `icp-cli` / `dfx` | Low (zero external canister calls) | **Phase 1 (MVP)** |
| **ICRC-2 `transfer_from` (ICP / ckUSDC) + CMC** | Works from web wallets (Plug, Oisy, II) | Medium (ICRC-2 approve + CMC `notify_mint_cycles` / swap) | **Phase 2** |

- **Fee model:** Charge a transparent creation setup fee in Phase 1, and **0% margin on raw cycle `top_up` pass-through** (since customers can bypass raw cycle fees via CMC directly). Monetize convenience in Phase 2 via a small conversion margin on one-click **ICP / ckUSDC → cycles** top-ups and optional managed upgrades.
- For Phase 2 ICP/ckUSDC payments, use the **deposit-then-create** or **ICRC-2 `approve` + `transfer_from`** pattern rather than direct transfers, so failed swaps/creations stay credited to the caller's principal inside `CREDITS` and can be withdrawn or retried.

### B. Controller & Upgrade Trust Model
- **Recommended default (`CustomerOnly`):** Customer is the sole controller from the moment installation completes (`controllers = [customer]`). Neither the factory nor its operators can inspect customer audit logs/policies or upgrade customer code. Customers upgrade via `icp-cli` / `dfx` or via the factory UI's client-side chunked upgrade helper (`post_upgrade` ignores its arguments, so state is untouched).
- **Explicit opt-in (`CoControlled`):** Customers who want automated/one-click server-side upgrades and cycle-health polling (`upgrade_instance`, `get_instance_health`) can opt into `[customer, factory]` at creation and call `renounce_factory_control` at any time (which reads live `canister_status` and removes *only* `factory`, preserving any other controllers the customer added).

### C. Safe Defaults for Newly Provisioned Instances
When the factory constructs `InitArgs.limits` (unless overridden by the customer):
- `max_calls_per_script: 25`
- `max_instructions: 20_000_000_000`
- `call_timeout_seconds: Some(30)` — prevents hung third-party canisters from blocking `stop_canister` during upgrades.
- `certified_reads: Some(false)` (or customer-selectable) — fast query reads for `initialize`/`tools/list`/`search`, with `certified_reads: Some(true)` available for high-assurance setups.
- `enable_demo: Some(false)` — disables `src/demo.html` at `GET /` on customer canisters so paid instances do not expose hardcoded `demo_store` benchmarks (returns `404` on `/`).

### D. Custom Domains & Endpoint Discovery
- Every spawned canister is immediately reachable at `https://<canister-id>.icp0.io/mcp` over the ICP HTTP gateway with zero DNS setup.
- Customers needing branded endpoints (e.g., `mcp.acme.com`) can register a custom domain via standard IC HTTP gateway custom-domain records pointing to their `<canister-id>`.

---

## 6. Security & Operational Risks Addressed in This Plan

1. **Pre-execution input validation:** Validate `token_sha256`, `allowlist` (non-empty methods, no duplicates, size cap), and `variant` before `create_canister` to avoid stranding cycles on a trapped `init`.
2. **Same-subnet chunk reuse + cross-subnet fallback:** Stage active release chunks once in `factory`'s chunk store (`store_canister = Some(factory)`) for fast same-subnet installs, falling back to per-target `upload_chunk` + `clear_chunk_store` only for cross-subnet CMC placement.
3. **Concurrency lock & cycle reclaim:** Lock in-flight installs with `ProvisionStatus::Installing`, cap concurrent installs via `max_concurrent_installs`, and support `delete_failed_instance` + `drain_cycles` so cycles are reclaimed before `delete_canister`.
4. **Zero-trust defaults & certified factory UI:** Default to `CustomerOnly` control, enforce controller-only allowlist mutations, serve the factory UI with full `ic-http-certification`, and keep instance management on the factory's origin to avoid II `derivationOrigin` principal mismatches.
5. **Code-mode sandbox hardening:** Before multi-customer launch, resolve the known runtime edge cases in `icp_code_mode` (Python `to_json` recursion depth bound, Boa microtask loop / `JSON.stringify` replacer bounds, Python `max_instructions` alignment, and per-HTTP-request batch limits for `search_icp_canisters`).
6. **Allowlist cache eviction & cycle-freeze prevention:** Ensure `remove_allowlist` / `remove_allowlists` / `replace_allowlist` clear `DID_CACHE` entries, disable `demo.html` on customer instances by default (`enable_demo: Some(false)`), and surface cycle health / CMC top-up guidance before `freezing_threshold`.

---

## 7. Phased Delivery Plan

- **Phase 1 — MVP Cycles Factory & Bulk Allowlist:**
  - `icp_code_mode_factory` canister with chunked admin release upload (`begin_release` / `upload_release_chunk` / `finalize_release`), same-subnet chunked installs (`store_canister = Some(factory)`), `create_instance` (`CustomerOnly` default, direct cycles + initial `allowlist`), `retry_install`, `delete_failed_instance`, `withdraw_credits`, `top_up`, `my_instances`, and `renounce_factory_control`.
  - Add validated batch allowlist methods (`set_allowlists`, `remove_allowlists`, `replace_allowlist` with `DID_CACHE` eviction), `drain_cycles`, and `enable_demo: Option<bool>` on `icp_code_mode`.
  - Integration tests on local `icp` / PocketIC verifying chunked release upload, `InitArgs` wiring, concurrent retry locking, failed-instance cycle reclaim, bulk allowlist updates, and state preservation across upgrades (with any upgrade arguments).
- **Phase 2 — Token Payments & Opt-In Managed Upgrades:**
  - ICRC-2 ICP / ckUSDC payment flow + CMC cycle minting + refundable credit ledger.
  - `upgrade_instance` workflow for `CoControlled` opt-in instances (`canister_status` sync → `stop_canister` → `install_chunked_code(upgrade, arg=None)` → `start_canister`).
- **Phase 3 — Certified Self-Serve UI & Trial Funnel:**
  - Fully certified (`ic-http-certification`) onboarding & management UI served from `factory` (in-browser token generator, variant picker, cycle calculator, **self-serve bulk allowlist editor with read-only presets, `.did` override upload, and diff preview**, instance decommission/drain, and MCP client config snippet).
  - Optional shared read-only free-trial canister (already isolated from customer canisters) for zero-friction evaluation.
