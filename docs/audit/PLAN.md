# Audit plan

A repeatable, whole-repository audit. One run should find the issues that exist at that
commit, prove each one, sort them, and say what would stop that kind of issue from coming
back.

This file is generic on purpose. It names no file, function, limit or count from the code,
so it stays valid as the app grows. Everything specific is derived from the repository at
the start of each run (Phase 1) or read from the findings register
([REGISTER.md](REGISTER.md)), which audits append to. Do not edit this plan to record
findings; record them in the register.

## 1. What "exhaustive" means here

No audit can prove the absence of bugs. This plan gets as close as a process can by
replacing "look around for problems" with four things that can be checked:

1. **A complete map.** Every entry point, input, piece of state, outbound effect, limit
   and documented claim is listed mechanically, not from memory.
2. **A complete grid.** Every item on the map is examined under every applicable review
   lens (Phase 3). An empty cell is unfinished work, not an assumption of safety.
3. **Variant sweeps.** Every confirmed finding is turned into a pattern and the whole
   codebase is searched for the same mistake elsewhere (Phase 6).
4. **A stopping rule.** The audit ends only after two consecutive rounds find nothing new
   (Phase 6), and it reports what it could not test.

A run is **done** when all of these hold:

- every inventory in Phase 1 was regenerated from the code at the audited commit;
- the coverage grid has no empty applicable cell;
- every finding in the register that is still open or fixed was re-checked;
- every confirmed finding has a reproduction, a severity, a class and a proposed fix;
- two consecutive discovery rounds produced no new confirmed finding;
- the report lists everything that was not tested, and why.

## 2. Ground rules

- **Evidence.** A finding counts only if it was reproduced (a failing test, a request and
  its response, a measurement) or the code path produces it unambiguously. State which.
  Suspicions go in the report as "unverified", never as findings.
- **Refute first.** Before reporting, try to disprove the finding. Record what was tried.
- **Do not change the audited tree.** Work in a private copy with its own build directory.
  The only files an audit writes in the repository are the run report and the register.
- **No stale binaries.** `git archive` keeps commit times, so a copy built in a shared
  target directory can be reported `Fresh` and run another copy's binary. After extracting,
  `touch` every source file (or give each copy a unique package version). The first build
  of a copy must show the workspace crates as `Compiling`, not `Fresh`; re-run any result
  obtained otherwise.
- **Destructive tests** (draining balances, filling storage, changing policy, upgrading)
  run only on a disposable local deployment created for the audit, never on an instance
  that holds real data or value.
- **Secrets.** Never print, log or commit credentials. Refer to them by where they live.
- **One root cause, one finding.** Symptoms that share a cause are listed under it.
- **No fixes during discovery.** Fixing changes the thing being measured. Fix after the
  report (Phase 8), then re-run the checks for what was touched.

### Severity

| Level | Meaning |
|---|---|
| Critical | A guarantee the product exists to provide is broken: an unauthorized effect, loss or theft of value, corrupted or lost durable state, or a build that cannot be deployed |
| High | A caller with ordinary access can make the service fail or cost money, return wrong data as if it were right, or hide or falsify a record meant to be trustworthy |
| Medium | Behaviour differs from the documentation or a specification, limits are inconsistent, or an operator can be misled |
| Low | Edge cases, inefficiency, and hardening with no demonstrated failure |

Grade by the worst outcome that was demonstrated, not the worst imaginable.

## 3. Phase 0 — Baseline

Record, in the run report:

- the commit, the date, the toolchain versions, and who or what ran the audit;
- that the project builds for every target and feature combination it supports;
- that the existing test suite passes, and how long it takes;
- for each deployable artifact: its size, its hash, and the external functions it imports
  (anything the platform does not provide makes it undeployable);
- a fresh local deployment made by following the project's own instructions exactly. Any
  step that fails or is missing from the instructions is itself a finding.

## 4. Phase 1 — Inventory

Regenerate every list below from the code. Use searches and tools, not recollection. Save
them in the run report: they are the rows of the coverage grid.

| Inventory | What to list | How to derive it |
|---|---|---|
| Components | Every deployable unit, library, script, page and build step | Workspace and build manifests, deployment config |
| Actors | Everyone who can interact: anonymous callers, authenticated users, operators, other services, infrastructure providers | Authentication and authorization checks; deployment docs |
| Entry points | Every externally callable function, route, message handler, lifecycle hook and timer, with who may call it and whether it can change state | Interface definitions, routing tables, handler attributes |
| Untrusted inputs | Every value that originates outside the trust boundary, including replies from other services and data fetched at run time | Follow each entry point's parameters and each outbound call's result |
| Outbound effects | Every call, message, payment or write that leaves the component | Search for the call and transfer primitives the platform offers |
| Durable state | Everything that survives restarts or upgrades, with its format and who can write it | Storage declarations; upgrade hooks |
| Volatile shared state | Globals, caches, counters and pools that outlive one request | Static and thread-local declarations |
| Commit points | Every place where execution can be suspended, state is committed, or a failure rolls work back | Await points, transaction boundaries, callbacks |
| Limits | Every constant or setting that bounds size, count, depth, time, memory or cost, and what it protects | Constants and configuration |
| Unbounded work | Every loop, recursion, allocation or copy whose size comes from an untrusted input | Follow untrusted inputs into loops, recursive functions and conversions |
| Failure sites | Every place that can abort: assertions, unwraps, indexing, arithmetic overflow, out-of-memory | Search for the language's aborting constructs outside tests. Also list the panic locations compiled into the release artifact, grouped by crate (the `.rs` path strings in its data section, e.g. `strings` or `wasm-objdump -s` and grep `\.rs`), and reconcile them with the source rows: dependencies abort too |
| Boundary conversions | Every translation between formats or type systems | Serializers, parsers, encoders, value mappers |
| Value flows | Every path by which money, credits, tokens or metered resources move or are spent | Payment, metering and resource-accounting code |
| Admin surface | Every setting an operator can change, its valid range, and what happens outside it | Configuration endpoints and init arguments |
| Supply chain | Dependencies, vendored or patched code, build flags, toolchain pins | Manifests, lockfiles, patch sections |
| Claims | Every sentence in the docs, interface descriptions and comments that promises behaviour ("never", "always", "every", "cannot", a number) | Read the documentation end to end |
| Tests | What each test actually asserts, and which inventory rows have no test | The test suite |
| History | Every past finding and its class | [REGISTER.md](REGISTER.md) |

Two checks on the inventory itself:

- **Diff it against the previous run's inventory.** New rows are where new bugs live;
  review them first.
- **Look for what is missing.** For each inventory, ask what kind of item would not have
  been caught by the method used to derive it, and search for that directly.

## 5. Phase 2 — Invariants

Write down what must always be true, then find where each is enforced and how it is tested.

Sources: every row of the Claims inventory, plus the generic invariants below, kept where
they apply.

1. **Authorization.** No effect happens on behalf of a caller who is not allowed to cause it.
2. **Policy precedes effect.** Every outbound effect is checked against policy before it
   happens, using the canonical form of every identifier.
3. **Records match reality.** Every record meant to be trusted (logs, receipts, balances)
   exists if and only if the thing it describes happened.
4. **Bounded cost.** No caller can make the service do unbounded work, hold unbounded
   memory, store unbounded data or send unbounded messages. The party that pays for work is
   the party that asked for it, or the cost is refused before it is incurred.
5. **Clean failure.** Every input ends in a well-formed response or a documented refusal,
   never in an abort that discards work or records.
6. **Isolation.** One request, user or tenant cannot observe or influence another's.
7. **Determinism.** Where the platform requires identical results across machines, nothing
   depends on wall-clock time, randomness, iteration order or machine state.
8. **Fidelity.** Data crossing a boundary arrives intact or fails loudly. It is never
   silently truncated, rounded, dropped or reinterpreted.
9. **Durability.** Upgrades, restarts and schema changes preserve durable state and keep
   old data readable.
10. **Honest interface.** The documentation, the interface definitions and the behaviour
    agree.

For each invariant, record: the exact statement for this codebase, where it is enforced,
which tests cover it, and its status (holds, violated, unverified). An invariant with no
enforcement point or no test is a finding.

## 6. Phase 3 — Review lenses

Review every inventory row under every lens that applies to it. Each lens is a question. A
lens applies when the inventory contains the thing it asks about. Classes recorded in the
register that are not listed here are additional lenses.

| # | Lens | The question | Look especially for |
|---|---|---|---|
| L1 | Access control | Who can reach this, and should they? | Missing or inconsistent checks between equivalent paths; checks on a different form of the identifier than the one used; privileged actions reachable through a less privileged feature; self-calls and calls to platform services |
| L2 | Atomicity and commit points | If execution stops here, what has already been committed and what is lost? | Records written in a different step from the effect they describe; cleanup that does not run; state that stays "in progress" forever; work done before validation |
| L3 | Resource bounds | What input makes this run longest, deepest, largest? | Loops, recursion and copies sized by input; limits checked after the work instead of before; limits on one dimension but not another (count but not bytes, depth but not width); work done inside a dependency where the app's own limits do not reach |
| L4 | Multiplication | What happens when this is done N times at once, or N times in one request? | Per-item budgets multiplied by batch size; per-request budgets multiplied by concurrency; retries and replays; fan-out to other services |
| L5 | Cost and payment | Who pays, and can someone else make them pay? | Work accepted before the caller is authenticated; storage that grows with unauthenticated or cheap requests; balances that can be spent twice, go negative or be stranded |
| L6 | Consistency across variants | Do all equivalent implementations enforce the same rules? | A limit present in one runtime, route, language or code path and absent in its sibling; a configured value honoured in one place and hard-coded in another |
| L7 | Data fidelity | Does this value survive the trip? | Precision loss, silent truncation, defaulting of unknown or misspelled fields, lossy round trips, success reported for partial data, different handling of the same value by different components |
| L8 | Concurrency and shared state | What if two of these interleave? | Globals read before and written after a suspension; caches poisoned by one request and used by another; counters that leak on failure; state assumed to be per-request that is per-process |
| L9 | Failure handling | What happens when a dependency is slow, wrong, hostile or absent? | Waiting forever; trusting replies, sizes or self-descriptions from another party; failures retried without limit; errors that leak internals or echo unbounded input |
| L10 | Trust and transport | Who can read or alter this in transit or at rest? | Secrets visible to infrastructure operators; responses that are not authenticated; content served to users that asks for credentials; anything an intermediary could change to steer a client or an AI model |
| L11 | Sandbox and determinism | Can untrusted code or input reach something it should not? | Dynamic evaluation, time, randomness, host access; state surviving between runs; escaping a wrapper; influence over a later run |
| L12 | Protocol conformance | Does it do what the specification says, including for malformed input? | Required fields, error codes, status codes, batching rules, identifier rules, behaviour on both happy and hostile paths |
| L13 | Upgrade and compatibility | Does old data, an old client or a half-finished operation survive a new version? | Schema changes, interface changes that break existing callers, volatile state lost on upgrade, operations in flight during an upgrade |
| L14 | Configuration | What can an operator set that breaks a guarantee? | Values outside a safe range that are accepted; settings that interact; defaults that are unsafe for production |
| L15 | Supply chain and build | Is what ships what was reviewed? | Patched or vendored code drifting from upstream; unpinned versions; build flags that change behaviour; artifacts that cannot be reproduced; young or unmaintained dependencies on the critical path |
| L16 | Claims | Is every promise true, and is it tested? | Absolute words, numbers and guarantees in documentation, interface descriptions, error messages and anything shown to end users or to an AI model |
| L17 | Client and presentation | Can the user-facing surface be turned against the user? | Injection into pages, credential handling in the browser, content that an AI model reads and may act on |
| L18 | Operations | Can this be run safely for a year? | Running out of funds or storage without warning; no way to observe failures; logs that grow without bound; no recovery path |

For each row and lens, the auditor writes one of: **finding**, **holds** (with the reason),
or **not applicable** (with the reason). "Looks fine" is not an entry.

## 7. Phase 4 — Dynamic testing

Reading finds what the code says. These find what it does. Apply each technique to every
inventory row it fits.

- **Boundary tests.** For every row in the Limits inventory, test just under, at, and just
  over the limit, and confirm what the documentation says happens. Then find the largest
  input that passes each limit and measure what it costs.
- **Property and fuzz tests.** For every row in Boundary conversions and every parser:
  random and adversarial inputs must never abort, and a round trip must return the original
  value or a clear error. Seed with every past finding from the register.
- **Differential tests.** Where two or more implementations should behave the same (runtimes,
  routes, languages, encoders), feed them the same inputs and compare results, errors and
  costs.
- **Fault injection.** At every row in the Commit points inventory, force a failure and
  inspect what was committed, what was recorded and what the caller saw. Also inject:
  a refused send, a dependency that never answers, one that answers late, one that returns
  oversized or malformed data, one that lies about itself, exhausted funds, exhausted
  storage, and an upgrade while work is in flight.
- **Concurrency tests.** Interleave requests at every suspension point: same user, different
  users, same resource, mixed kinds. Check isolation, counters and caches afterwards.
- **Abuse tests.** For every entry point, send the largest, deepest, most numerous and most
  malformed input the transport allows, with and without valid credentials. Measure cost to
  the service and to the caller.
- **Scale tests.** Fill each durable store to a size that represents a year of use and
  confirm reads, pagination, upgrades and costs still work.
- **Deployment tests.** Whatever was verified only on a local or simulated environment is
  listed as unverified until it is repeated on the real platform or a faithful staging copy.

Any behaviour that cannot be tested with the project's current tooling is recorded, together
with the test harness that would make it testable. Missing harnesses are findings of their
own (class: testability).

## 8. Phase 5 — Verification

Every candidate finding goes to a second reviewer who did not find it, whose job is to
refute it:

1. Read the code path independently. Is the claimed behaviour possible?
2. Reproduce it. If it cannot be reproduced, it is refuted or downgraded to "unverified".
3. Re-grade the severity against Section 2.
4. Find the root cause and merge duplicates.
5. Record the evidence: the exact input, the observed output, and where it was run.

## 9. Phase 6 — Completeness

- **Variant sweep.** For each confirmed finding, state the mistake as a pattern ("a record
  written before the action it describes", "a limit enforced in one variant only"). Search
  every inventory row for the same pattern. Each hit is a new candidate finding.
- **Coverage grid.** Build the grid of inventory rows against lenses. Fill every applicable
  cell. Send the empty and weakest cells back through Phases 3 to 5.
- **Completeness critic.** A reviewer who has seen only the inventories, the grid and the
  findings answers: which rows were examined least, which lens produced nothing and why,
  which claim has no test, which past class from the register found nothing this time, and
  what an attacker would try that nobody tried. Their answers become the next round's work.
- **Stopping rule.** Repeat discovery rounds until two in a row confirm nothing new.

## 10. Phase 7 — Report and register

Write one report per run in `docs/audit/runs/`, named by date and commit. It contains:

1. **Summary:** the verdict in a few sentences, counts by severity, and the three most
   important things to fix.
2. **Baseline** (Phase 0) and **inventories** (Phase 1), with the diff against the last run.
3. **Invariants** and their status.
4. **Findings**, most severe first. Each has: an id, a title, the class, the severity, the
   invariant it violates, where it is, how to reproduce it, the evidence, the root cause,
   the proposed fix, and the regression test that will keep it fixed.
5. **Refuted candidates**, with why.
6. **Critique.** Beyond individual bugs: which design choices keep producing findings, what
   should be restructured instead of patched, and which guarantees the product should stop
   making or start making.
7. **One step further.** For each class that produced findings: the test, tool, type or
   design change that would make that class impossible or automatically detected. Rank these
   by how many past findings each would have caught.
8. **Not tested:** everything the audit could not check, why, and what would make it
   checkable.
9. **Coverage grid.**

Then update [REGISTER.md](REGISTER.md): add each confirmed finding with its class, mark
re-checked past findings as still fixed or regressed, and add any new class.

## 11. Phase 8 — Close-out

Fixing is separate from auditing. For each finding, in severity order:

1. Write the regression test first and watch it fail.
2. Fix the root cause where all callers pass through, not the reported symptom.
3. Re-run the variant sweep for that class.
4. Update every claim in the documentation that the fix changes.
5. Mark the finding fixed in the register, with the commit and the test.

After all fixes, re-run Phases 0, 4 and 6 for everything that was touched.

## 12. How to run it

**As a person or a single agent:** follow Phases 0 to 7 in order. Start with this prompt:

> Audit this repository by following docs/audit/PLAN.md exactly. Derive every inventory
> from the code. Reproduce every finding before reporting it. Do not fix anything. Write
> the report to docs/audit/runs/ and update docs/audit/REGISTER.md.

**As a multi-agent workflow:** the saved workflow `audit` runs the same phases with
independent reviewers per lens, a separate verifier for each, variant sweeps, a completeness
critic and the stopping rule. Ask for it by name ("run the audit workflow"). It takes an
optional depth:

| Depth | What runs | Use it for |
|---|---|---|
| `quick` | Inventory, the lenses the inventory diff touches, one round | After a small change |
| `full` (default) | Everything in this plan, up to the stopping rule | Before a release, or periodically |

**When to run:** before any release that handles value or new kinds of users, after any
change to authorization, metering, storage or an external interface, after a dependency or
toolchain upgrade, and on a schedule.

## 13. Keeping this plan valid

- The plan describes method. It should change only when the method changes.
- The register holds everything specific: findings, classes, and the regression list.
- If a finding does not fit any lens, record its class in the register. The next audit
  treats every class in the register as a lens, so nothing here needs editing.
- If an inventory in Phase 1 keeps missing a kind of item, that is a method change: add the
  derivation here.
