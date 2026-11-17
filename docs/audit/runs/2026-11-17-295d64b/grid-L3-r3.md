# Coverage grid: L3 Resource bounds, round 3 (Monty native operations whose cost grows with input)

Run `2026-11-17-295d64b`. Row ids refer to `inventory.md`. Cells: **finding** (candidate id
below), **holds** (why), **n/a** (why). Candidates L3r3-1 and L3r3-2 are described in the lens's
returned findings. Known and not repeated: bigint `**` (`long_int.rs:1054-1069, 1166`),
`py_repr`/repr of big ints in `to_json`, `eval`/`exec` compiling runtime-built source.

## How it was tested

- **Where.** The project's local icp-cli network (port 8010), canister `icp_code_mode`, module
  `224cfa7a...` (built from the committed tree, see `grid-L13-r2.md`). Limits at their defaults
  throughout (`max_instructions` 20e9; confirmed by `while True: pass` ending at
  `20000533166 > 20000000000`). No setting was changed; nothing was deployed.
- **How measured.** Each probe is one `execute_icp_script` (`language: python`) with a unique
  trailing comment. Its audit entry is found by `script_sha256` (other agents were writing
  entries at the same time) and its `instructions` field read with `get_audit_logs`. The cost of
  an operation is the run's total minus a setup-only run. Helper:
  `$SCRATCH/l3r3/probe.sh` (token read from `.env`, never printed).
- **Interruptibility test ("PRE").** Setup, then `for i in range(23500000): pass` (815
  instructions per iteration; the loop alone ends at 19.16e9), then the operation. A polled
  operation stops just past the budget (`> 20000000000` by under 0.1e9). An unpolled one runs to
  completion and the post-run check reports budget + its whole cost.
- **Trap test.** An operation that costs more than about 20e9 after the PRE loop, or more than
  40e9 on its own, ends in HTTP 503 `Canister exceeded the limit of 40000000000 instructions for
  single message execution`. No audit entry is written (nothing was sent; register #11).

## Per-operation results (wasm, live)

Largest input that fits the 64 MiB limit unless a smaller one already traps.

| Operation | Input | Instructions in the one call | Polled inside? | Outcome |
|---|---|---|---|---|
| `math.gcd(a, b)` | 2^16 / 2^17 bits | 4.65e9 / 18.4e9 (quadratic) | no | 2^18-bit operands (32 KiB each): **trap** (L3r3-1) |
| `math.lcm` | (calls `gcd`, `math.rs:1104-1105`) | same as gcd | no | same (code path) |
| `math.isqrt(a)` | 2^20 bits | 14.8e9 | no | 2^21 bits (256 KiB): **trap** (L3r3-1) |
| `a * b` (random) | 2^20 / 2^22 bits | 0.91e9 / 20.6e9 | no (PRE overshoot = full cost) | 2^23 bits (1 MiB): **trap** (L3r3-1) |
| `a // b`, `a % b` (random) | 2^20//2^19 / 2^22//2^21 bits | 2.26e9 / 18.4e9 | no | 2^23//2^22 bits (1 MiB by 512 KiB): **trap** (L3r3-1) |
| `pow(a, b, m)` | m 2^20 / 2^21 bits, e=3 | 9.3e9 / 25.7e9 | per exponent bit only | m 2^22 bits (512 KiB): **trap**; one squaring+reduction is unpolled (L3r3-1) |
| `decimal` sqrt | `getcontext().prec` 1e5 / 3e6, `Decimal(2).sqrt()` | 0.65e9 overshoot / - | between iterations only | prec 3e6, no input data at all: **trap** (L3r3-1) |
| `re.match('a*(?=[^x]*$)', 'a'*N+'x')` | N 1e4 / 3e4 | 1.23e9 / 10.9e9 (quadratic) | no (PRE overshoot 10.9e9) | N = 1e5 (100 KB): **trap** (L3r3-1) |
| `re.match('(a|a)*b', 'a'*N)` (delegated, linear, ~383 instr/char) | N 1e6 / 1e7 / 6e7 | 0.385e9 / 3.8e9 / 23.0e9 | no (PRE 5e7: 38.3e9 total) | N = 6e7 after the PRE loop: **trap** (L3r3-1) |
| fancy-regex `backtrack_limit` (1,000,000) | `(a|a)*(?=b)`, `(a|a)*\1b` on 'a'*30 | 0.90e9 / 0.83e9 per call (~850 instr per backtrack step) | n/a | `re.PatternError` "Max limit for backtracking count"; the limit bounds steps, not work per step |
| `unicodedata.normalize('NFKC', 'ﷺ'*1.5e6)` (4.5 MB) | | 14.2e9 then MemoryError at 71.6 MB | no (PRE 33.4e9 total) | overshoot 14.2e9 (L3r3-2) |
| `int(s, 16)` | 30M hex digits | 10.6e9 | no (PRE 29.97e9) | overshoot (L3r3-2) |
| `str.zfill` / `ljust` / `center` | width 6e7 | 11.0e9 each | no (PRE 29.87e9) | overshoot (L3r3-2) |
| `str.replace('a','bb')` | 20 MB -> 40 MB | 8.4e9 | no (PRE 27.3e9) | overshoot (L3r3-2) |
| `bytes.hex()` | 20 MB | 6.2e9 | no (PRE 25.2e9) | overshoot (L3r3-2) |
| `t in l`, `l.index(t)`, `l.count(t)` | `l = [s]*1e6`, s and t distinct 16 MB strings differing in the last byte | | every 64 items (`LOOP_CHECK_INTERVAL`) | overshoot 6.9e9 (64 x 16 MB compares) (L3r3-2) |
| `int(str)` base 10 | 4300 / 4301 digits | 5.1e6 / ValueError | n/a | holds (`INT_MAX_STR_DIGITS`, `type.rs:1183-1186`) |
| `'%d' % x`, `format(x,'d')`, `f'{x:,}'` | x 2^20 bits | ValueError | n/a | holds (same digit limit) |
| `Decimal(str)` | 1e5 / 1e7 digits | ValueError (2.9e9 to scan 1e7) | n/a | holds (4300-digit coefficient limit) |
| `str * n` | 60 MB | 0.38e9 | n/a | holds (cheap; size prechecked) |
| `','.join(l)` | 1e6 items | 1.3e9 | yes (PRE 20.0007e9) | holds |
| `str.split(',')` | 'a,'*5e6 / 16e6 | MemoryError at 77 MB / 99 MB | yes (periodic memory check) | holds; memory overshoots 64 MiB by up to ~1.5x before the check |
| `sorted(l)` | 1e6 shuffled ints | ~5.7e9 | yes (PRE 20.0008e9) | holds |
| `sorted([s]*1e6)` | 16 MB string | MemoryError (merge buffer) | n/a | holds |
| nested list `in` | `[[s]*1000]*1000` | | yes (20.01e9) | holds |
| `math.factorial` / `comb` / `perm` | 3e5, 1e6 / (1e6, 5e5) | stopped at 22.2e9 / 23.2e9 / 20.02e9 / 20.03e9 | between products (overshoot <= 3.2e9) | holds at the default budget |
| `'{:>N}'.format`, f-string width | N 6e7 / 3e9 | 7.46e9 to MemoryError | yes (PRE 20.0007e9) | holds |
| `'%3000000000s' % 'a'` | | ValueError "width too big" | n/a | holds |
| `hashlib.sha256` | 60 MB | ~5.4e9 | yes (PRE 20.004e9) | holds |
| `json.dumps` / `json.loads` | 1e6 ints / 6.9 MB | 1.5e9 / polled (PRE 20.0008e9) | yes | holds |
| `re.compile('(?:a{1000}){1000}')` | | 0.46e9 to PatternError (NFA size) | n/a | holds |
| `str.find` | 'a'*1e6 find 'a'*5e5+'b' | 0.11e9 | n/a | holds (linear search) |

**Is the 64 MiB limit applied before one large allocation?** Yes, for every single large
allocation tried: `'a'.ljust(3e9)`, `'1'.zfill(3e9)`, `b'a'.ljust(3e9)`, `bytes(3e9)`,
`random.randbytes(3e9)`, `random.getrandbits(2.4e10)`, `int(Decimal('1e1000000000'))`,
`f"{'a':>3000000000}"` all raise `MemoryError: memory limit exceeded: ~3000032849 bytes >
67108864 bytes` (or a size error) within 0.7e6-1.1e6 instructions; `[0]*3e9` raises a bare
`MemoryError`; `'\t'.expandtabs(3e9)` raises OverflowError. No wasm out-of-memory trap.
(`bytearray` and `int.to_bytes` do not exist in Monty.) Per-item growth inside one native call is
checked periodically, so memory can exceed 64 MiB by up to ~1.5x before the error (`split`:
99 MB, `list(s)`: 80 MB, NFKC: 82 MB).

## Grid

| Row | Cell | Reason |
|---|---|---|
| W10 Python parse/compile, `to_json`, `from_json` | finding L3r3-1, L3r3-2 (VM side); known for `to_json` repr and `exec` compile | The VM's own native operations are the unbounded work: see the table. `to_json`/`from_json` host conversion: known (SW2-1-1), not repeated |
| LIM26 Python limits (VM budget, turn <= 30e9, 64 MiB) | **finding L3r3-1** | The VM budget is polled between bytecodes and inside some loops; the operations above run to completion. With default settings 7 different operations trap at 40e9 from inputs of 32 KiB to 1 MiB, or with no input (decimal precision) |
| LIM26 memory 64 MiB | holds | Precheck before every single large allocation tried (above). Periodic checks bound per-item growth to ~1.5x the limit, far below the 3 GiB wasm memory |
| LIM26 recursion 200, suspensions 256 | n/a | Not a native-op cost dimension (covered in grid-L3-r1) |
| LIM15 MAX_LOG_BYTES (Python print) | n/a for this question | Print formatting cost is known (formatted in full before the cap); `print` of a big int is subject to the 4300-digit limit (ValueError, measured for `%d`/`format`) |
| LIM16 value limits | n/a for this question | Host-side conversion, known (SW2-1-1). Native ops act inside the VM before any value leaves it |
| F13 out of memory | holds (Python) | No OOM trap reachable: every large single allocation is refused before it is made; periodic overshoot <= ~100 MB |
| F14 40e9 per-message limit | **finding L3r3-1** | Reproduced traps: gcd, isqrt, `*`, `//`, 3-arg pow, decimal sqrt, regex (quadratic lookahead; linear delegated match over 60 MB after the budget is nearly spent). **L3r3-2**: unpolled linear ops overshoot the policy budget by up to 14.2e9, so `max_instructions` above ~25.8e9 makes them trap too |
| CL (README.md:371 "The VM stops when the policy's instruction budget is spent"; README.md:454-458 "Every other path stops with a clean error") | finding L3r3-1, L3r3-2 | Measured 33.4e9 used against a 20e9 budget (NFKC) and 503 traps (above). The claim violation itself is already filed with the bigint `**` finding; these are further triggers |
| I4 Bounded cost / I5 Clean failure | violated (L3r3-1) | |

## Not tested

- Every Monty built-in. The enumeration covered the categories named in the task plus gcd/lcm,
  isqrt, decimal, unicodedata, hashlib, json, sort, list search, padding/format and large single
  allocations. Not measured: `itertools` (product/permutations are lazy, polled by the VM),
  `csv`, `tomllib`, `base64`, `datetime`, `fnmatch` (translates to a regex, so the regex result
  likely applies), `str.translate`, `str.title`, `difflib` (absent).
- `math.lcm` trap: code path only (it calls `BigInt::lcm` -> `gcd`).
- Exact instruction count of the trapping calls (they abort; only the >40e9 fact is known).
- Mainnet: the local replica's instruction accounting is the same metering, but cycle cost per
  trap was not measured.
