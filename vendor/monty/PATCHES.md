# Patched monty 1.6.0

A copy of `monty` 1.6.0 from crates.io (`src/`, `README.md`, and `Cargo.toml` without
its test targets and dev-dependencies), used through `[patch.crates-io]` in the root
`Cargo.toml`. `monty-types` still comes from crates.io. Every change is marked
`// icp-code-mode patch:` in the source.

Why: Monty checks its time limit only between bytecodes (every 255 of them) and in some
loops. A native operation whose cost grows faster than its input (big-integer `*`,
`//`, `math.gcd`, `decimal` at high precision, a backtracking regex, a probe through
colliding hash keys, compiling run-time text) is one step the limit cannot interrupt,
and a loop of them runs dozens past it. On the IC that ends in the 40B-instruction trap,
which discards the audit entry (audit findings 60, 61, 62 and 93).

## Changes

| File | Change |
|---|---|
| `src/resource_checks.rs` | `MAX_INT_BITS = 2^15` and `check_int_bits` (catchable `OverflowError`). `check_pow_size`, `check_mult_size`, `check_lshift_size`, `check_product_size` and `check_div_size` refuse results (or dividends) over the cap and return `RunResult`. `check_int_bits` also polls the time limit for operands over 2^12 bits, and `check_estimated_size` polls it before any result over 100 KB. |
| `src/types/long_int/arithmetic.rs` | `%` on big ints goes through `check_div_size` (it had no check). |
| `src/types/long_int.rs` | `pow(b, e, m)`: modulus capped; `num-bigint`'s Montgomery `modpow` only for moduli up to 4096 bits, the polled loop above (Montgomery setup alone took ~100 ms natively at 2^16 bits, whatever the exponent). True division keeps a memory-only check: its quotient has 55 bits, so it is linear. |
| `src/modules/math.rs` | `gcd`, `lcm`, `isqrt` operands capped (they skip the arithmetic checks, and additions can grow an int past the cap a bit per step). |
| `src/types/type.rs` | `int(text, base)` for power-of-two bases capped by significant digits x bits per digit (CPython's 4300-digit limit skips these bases). |
| `src/types/decimal/context.rs` | `check_digits` refuses more than 65,536 digits (it only checked memory) and polls time above 1,024. |
| `src/types/decimal/py_context.rs` | Settable precision capped at 10,000. |
| `src/types/re_pattern.rs` | `backtrack_limit` 10,000 (fancy-regex default 1,000,000). A regex that runs on fancy-regex's backtracking VM takes at most 5,000 bytes of subject; regexes the regex crate runs alone take any subject. Detected per compiled regex through `debug_print` (the anchored `match`/`fullmatch` variants can be backtracking when the plain one is not). Polls time before such a search over 1 KiB. |
| `src/types/hash_table.rs`, `src/types/set.rs` | A probe that walks more than 64 equal-hash entries polls the time limit (`set(keys)` over keys whose `__hash__` is constant is quadratic inside one bytecode). |
| `src/string_builder.rs` | Polls time when a string build grows past 64 KiB (`unicodedata.normalize` in a loop ran ~1.6 s past a 0.3 s limit). |
| `src/builtins/eval_exec.rs` | `snippet_source`, the choke point of `eval`, `exec` and annotation evaluation (`eval_hint`), refuses text over 16 KiB (the host's script limit) and polls time above 1 KiB. |
| `src/types/forward_ref.rs` | `ForwardRef.evaluate()` raises `NotImplementedError`: it compiles run-time text like `eval`, which the sandbox disables. |
| `src/builtins/print.rs` | Print probe: before formatting its arguments, `print` writes an empty string; if the writer refuses it, the call returns without formatting. The host's `Printed` callback refuses empty writes once its 64 KiB is full, so later prints cost nothing. Other writers accept empty writes. |
| `src/run_progress.rs` | `tracker_mut()` on `NameLookup`, `OsCall` and `ResolveFutures` (only `FunctionCall` had it), so the host can give the VM a fresh allowance from the live instruction counter before every resume. |

## Measurements (native, release, Apple M-series)

Single operations before the caps: `math.gcd` on two random ints of 2^15 / 2^16 / 2^17 /
2^19 bits took 4.3 / 17 / 70 / 4,200 ms; `isqrt` 1.1 ms at 2^15, 120 ms at 2^19;
`pow(a, 3, m)` with odd m 25 ms at 2^15, 103 ms at 2^16; `*` and `//` stay under 2 ms
at 2^17. `3 ** 4000000` took 163 ms natively and ~29e9 instructions on-chain (audit),
so bigint work costs up to ~180M Wasm instructions per native ms: the cap of 2^15 bits
puts the worst capped operation (`gcd`, ~4 ms) at well under 1e9.

Overshoot past a 300 ms time limit with each operation in a `while True` loop, after the
patches: `gcd` at the cap 1.6 ms (was 95 ms), `isqrt` 0.2 ms, `int(Decimal('1e65000'))`
1.1 ms (was 74 ms), `unicodedata.normalize` of 1.8M chars 6 ms (was 1.6 s), the
backtracking regex `a*(?=[^x]*$)` anchored on 10,000 bytes 4 ms (was 130-170 ms; one search
is n^2/2 steps, ~14 ms at 10,000 bytes, ~3.5 ms at the 5,000-byte cap), `set()` of 20,000
keys with a constant hash 0.2 ms, `exec` of 16 KB text ~1 ms.

## Updating

Copy the new release over `src/`, then re-apply the marked hunks (`grep -rn
'icp-code-mode patch' src`). `cargo test --lib runtimes::python` covers each one
(`engine_work_is_bounded`, `expensive_operations_poll_the_time_limit`,
`print_past_the_cap_is_not_formatted`, `host_time_is_deducted_from_the_vm_budget`).
