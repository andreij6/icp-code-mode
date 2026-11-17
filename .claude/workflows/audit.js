export const meta = {
  name: 'audit',
  description: 'Whole-repository audit following docs/audit/PLAN.md: inventory, a review per lens, independent verification, variant sweeps, a completeness critic, and a written report',
  whenToUse: 'Before a release, or after changes to authorization, metering, storage or an external interface. Optional args: {depth: "quick" | "full", focus: "free text"}. A full run uses dozens of agents.',
  phases: [
    { title: 'Inventory', detail: 'baseline, inventories and applicable lenses, derived from the code' },
    { title: 'Review', detail: 'one independent reviewer per lens' },
    { title: 'Verify', detail: 'a second reviewer tries to refute each finding' },
    { title: 'Sweep', detail: 'search the whole codebase for each confirmed pattern' },
    { title: 'Critic', detail: 'what was examined least; decides whether another round is needed' },
    { title: 'Live', detail: 'reproduce on a disposable deployment what needs one' },
    { title: 'Report', detail: 'write the run report and update the register' },
  ],
}

// Nothing about the application is written here. The plan and the register are read at
// run time, so this script needs no change when the app, the plan or the register changes.
const PLAN = 'docs/audit/PLAN.md'
const REGISTER = 'docs/audit/REGISTER.md'
const depth = (args && args.depth) === 'quick' ? 'quick' : 'full'
const focus = (args && args.focus) || ''
const MAX_ROUNDS = depth === 'quick' ? 1 : 5
const MAX_SWEEPS = depth === 'quick' ? 3 : 12

const RULES = `
You are part of an audit of the repository in the current working directory.
Read ${PLAN} first and follow it exactly; it defines the phases, the lenses, the evidence
standard and the severity scale. Read ${REGISTER} for past findings and known patterns.
Rules that always apply:
- Do not modify the repository, except files under docs/audit/ when your task says to.
- For anything you compile or run, copy the repository without its build output into a
  private temporary directory, build there with its own build directory, and delete it
  when you finish. Never build in the working directory itself.
- Do not deploy, upgrade, change settings on, or send destructive requests to any running
  instance unless your task is the Live phase.
- Never print or copy credentials. Refer to them by location.
- Report only what you reproduced or what the code path produces unambiguously, and say
  which. Do not fix anything.
${focus ? `The person running this audit asked for extra attention to: ${focus}` : ''}`

const INVENTORY_SCHEMA = {
  type: 'object',
  properties: {
    run_dir: { type: 'string', description: 'directory created under docs/audit/runs/ for this run' },
    commit: { type: 'string' },
    baseline_ok: { type: 'boolean' },
    baseline_notes: { type: 'string' },
    lenses: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          key: { type: 'string', description: 'lens id from the plan, or a class name from the register' },
          title: { type: 'string' },
          applies: { type: 'boolean' },
          touched_since_last_run: { type: 'boolean' },
          rows: { type: 'string', description: 'which inventory rows this lens should examine' },
        },
        required: ['key', 'title', 'applies', 'touched_since_last_run', 'rows'],
      },
    },
  },
  required: ['run_dir', 'commit', 'baseline_ok', 'baseline_notes', 'lenses'],
}

const FINDINGS_SCHEMA = {
  type: 'object',
  properties: {
    findings: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          title: { type: 'string' },
          lens: { type: 'string' },
          severity: { type: 'string', enum: ['critical', 'high', 'medium', 'low'] },
          location: { type: 'string' },
          invariant: { type: 'string', description: 'which invariant from the plan it violates' },
          scenario: { type: 'string', description: 'concrete input or state, and the wrong outcome' },
          evidence: { type: 'string', enum: ['reproduced', 'code_path'] },
          repro: { type: 'string' },
          pattern: { type: 'string', description: 'the mistake stated generally, for the variant sweep' },
          fix: { type: 'string' },
        },
        required: ['title', 'lens', 'severity', 'location', 'invariant', 'scenario', 'evidence', 'repro', 'pattern', 'fix'],
      },
    },
    grid_file: { type: 'string', description: 'file written in the run directory with this lens\'s coverage cells' },
    not_tested: { type: 'string' },
  },
  required: ['findings', 'grid_file', 'not_tested'],
}

const VERDICTS_SCHEMA = {
  type: 'object',
  properties: {
    verdicts: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          title: { type: 'string' },
          real: { type: 'boolean' },
          severity: { type: 'string', enum: ['critical', 'high', 'medium', 'low'] },
          evidence: { type: 'string', description: 'what was run or read, and what was observed' },
          root_cause: { type: 'string' },
          pattern: { type: 'string' },
          fix: { type: 'string' },
          regression_test: { type: 'string' },
          needs_live: { type: 'boolean', description: 'true if it can only be confirmed on a deployed instance' },
          location: { type: 'string' },
        },
        required: ['title', 'real', 'severity', 'evidence', 'root_cause', 'pattern', 'fix', 'regression_test', 'needs_live', 'location'],
      },
    },
  },
  required: ['verdicts'],
}

const CRITIC_SCHEMA = {
  type: 'object',
  properties: {
    gaps: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          key: { type: 'string' },
          title: { type: 'string' },
          rows: { type: 'string', description: 'what to examine, and the question nobody asked' },
        },
        required: ['key', 'title', 'rows'],
      },
    },
    assessment: { type: 'string' },
  },
  required: ['gaps', 'assessment'],
}

const REPORT_SCHEMA = {
  type: 'object',
  properties: { report_file: { type: 'string' }, summary: { type: 'string' } },
  required: ['report_file', 'summary'],
}

// ---------- Inventory ----------
phase('Inventory')
const inv = await agent(`${RULES}

Task: Phases 0 and 1 of the plan, then choose the lenses.
1. Create a directory docs/audit/runs/<date>-<short commit>/ (get both from the shell).
2. Do the baseline (Phase 0) and write it to baseline.md there.
3. Regenerate every inventory in Phase 1 from the code and write them to inventory.md
   there. If an earlier run directory exists, add a section listing what is new or changed.
4. List the invariants (Phase 2) for this codebase in invariants.md: the statement, where
   it is enforced, what tests it, and its status.
5. Return the lenses to run: every lens in the plan plus every class in the register that
   the plan does not list. For each, say whether it applies to what the inventory found,
   whether the code it covers changed since the last run, and which inventory rows it
   should examine.`, { label: 'inventory', phase: 'Inventory', schema: INVENTORY_SCHEMA })

if (!inv) return { error: 'the inventory agent did not finish; nothing was audited' }
const RUN = inv.run_dir
let lenses = inv.lenses.filter((l) => l.applies)
if (depth === 'quick') {
  const all = lenses.length
  lenses = lenses.filter((l) => l.touched_since_last_run)
  log(`quick depth: reviewing ${lenses.length} of ${all} applicable lenses (those covering changed code)`)
}
log(`commit ${inv.commit}; baseline ${inv.baseline_ok ? 'ok' : 'FAILED: ' + inv.baseline_notes}; ${lenses.length} lenses`)

// ---------- Review and verify, repeated until two rounds find nothing new ----------
const confirmed = []
const refuted = []
const notTested = []
const unverified = [] // candidates whose verifier did not finish; carried to the report
// Identity is the full normalised location and root cause; a key that matches an earlier
// finding is recorded as a merge on that finding, never dropped silently.
const seen = new Map()
const keyOf = (v) => `${v.location}|${v.root_cause}`.toLowerCase().replace(/\s+/g, ' ').trim()
const addConfirmed = (v, r) => {
  const k = keyOf(v)
  const existing = seen.get(k)
  if (existing) {
    if (existing.title !== v.title) {
      ;(existing.merged = existing.merged || []).push(v.title)
      log(`round ${r}: "${v.title}" merged into "${existing.title}" (same location and root cause)`)
    }
    return false
  }
  seen.set(k, v)
  confirmed.push(v)
  return true
}

const review = (lens, round) =>
  agent(`${RULES}

Task: Phases 3 and 4 of the plan for ONE lens: ${lens.key} — ${lens.title}.
Read ${RUN}/inventory.md and ${RUN}/invariants.md. Examine: ${lens.rows}
For every inventory row this lens applies to, decide: finding, holds (say why), or not
applicable (say why), and write those cells to ${RUN}/grid-${lens.key}-r${round}.md.
Use the dynamic techniques in Phase 4 that fit: boundary tests on limits, property or fuzz
tests on converters, differential tests between sibling implementations, fault injection.
Already reported, do not repeat: ${confirmed.slice(0, 200).map((c) => `${c.title} (${c.location})`).join(' ; ') || 'nothing yet'}
Return every candidate finding, and what you could not test and why.`,
    { label: `review:${lens.key}:r${round}`, phase: 'Review', schema: FINDINGS_SCHEMA })

// A null result (the agent was skipped or died) is a failure, not an empty result: absorb
// counts it so the round is not dry; a failed lens or pattern is tried again next round.
const verify = async (found, lens, round) => {
  if (!found) return null
  if (found.not_tested) notTested.push(`${lens.key}: ${found.not_tested}`)
  if (!found.findings.length) return { verdicts: [] }
  const verdicts = await agent(`${RULES}

Task: Phase 5 of the plan. Another reviewer (lens ${lens.key}) reported the candidates
below. You did not find them; your job is to refute them. For each: read the code path
yourself, try to reproduce it, re-grade the severity, find the root cause, and state the
general pattern. Mark real only if you reproduced it or the code path unambiguously
produces it. Say whether confirming it needs a deployed instance.
Candidates:
${JSON.stringify(found.findings, null, 1)}`,
    { label: `verify:${lens.key}:r${round}`, phase: 'Verify', schema: VERDICTS_SCHEMA })
  if (!verdicts) unverified.push(...found.findings.map((f) => ({ ...f, lens: lens.key, round })))
  return verdicts
}

const absorb = (results, r, what) => {
  let fresh = 0
  let failed = 0
  for (const res of results) {
    if (!res) { failed++; continue }
    for (const v of res.verdicts || []) {
      if (!v.real) { refuted.push({ title: v.title, why: v.evidence.slice(0, 400) }); continue }
      if (addConfirmed(v, r)) fresh++
    }
  }
  if (failed) log(`round ${r}: ${failed} ${what} agent(s) returned nothing, so the round does not count as dry`)
  return { fresh, failed }
}

let work = lenses
let dry = 0
let round = 0
let stopped = 'when the critic found no gaps'
const sweptPatterns = new Set()
// Patterns not yet swept (new, over the cap, or whose sweep failed) keep the loop going.
const unswept = () => [...new Set(confirmed.map((c) => c.pattern))].filter((p) => !sweptPatterns.has(p))
while ((work.length || unswept().length) && dry < 2 && round < MAX_ROUNDS) {
  round++
  const r = round
  const results = await pipeline(work, (lens) => review(lens, r), (found, lens) => verify(found, lens, r))
  const retry = work.filter((_, i) => !results[i])
  let { fresh, failed } = absorb(results, r, 'review/verify')

  // Variant sweep: each newly confirmed pattern is searched for across the whole codebase.
  const patterns = unswept()
  const toSweep = patterns.slice(0, MAX_SWEEPS)
  if (patterns.length > toSweep.length) log(`round ${r}: ${patterns.length - toSweep.length} patterns not swept this round (cap ${MAX_SWEEPS}); they carry over`)
  toSweep.forEach((p) => sweptPatterns.add(p))
  const sweeps = await pipeline(
    toSweep,
    (pattern, _p, i) => agent(`${RULES}

Task: variant sweep (Phase 6 of the plan). A confirmed finding reduces to this pattern:
"${pattern}"
Search every row of ${RUN}/inventory.md for the same mistake anywhere else in the codebase.
Do not re-report these known findings: ${confirmed.map((c) => c.title).join(' ; ')}
Write your cells to ${RUN}/grid-sweep-r${r}-${i}.md and return each new candidate.`,
      { label: `sweep:r${r}:${i}`, phase: 'Sweep', schema: FINDINGS_SCHEMA }),
    (found, _p, i) => verify(found, { key: `sweep-${i}` }, r))
  toSweep.forEach((p, i) => { if (!sweeps[i]) sweptPatterns.delete(p) })
  const s = absorb(sweeps, r, 'sweep/verify')
  fresh += s.fresh
  failed += s.failed

  log(`round ${r}: ${fresh} new confirmed finding(s); ${confirmed.length} total, ${refuted.length} refuted`)
  dry = fresh === 0 && failed === 0 ? dry + 1 : 0
  if (dry >= 2) { stopped = 'after two dry rounds'; break }
  if (r >= MAX_ROUNDS) { stopped = 'at the round cap'; break }

  // Completeness critic: decides what the next round looks at.
  const critic = await agent(`${RULES}

Task: completeness critic (Phase 6 of the plan). Read ${RUN}/inventory.md,
${RUN}/invariants.md and every grid-*.md file in ${RUN}. Findings confirmed so far:
${confirmed.map((c) => `- [${c.severity}] ${c.title}`).join('\n') || '(none)'}
Answer: which inventory rows were examined least, which lens found nothing and whether
that is believable, which claim has no test, which past class in the register found
nothing this time, and what an attacker would try that nobody tried. Return the specific
gaps to examine next, each as a lens key, a title, and exactly what to look at. Return an
empty list only if you would stake your reputation on the coverage.`,
    { label: `critic:r${r}`, phase: 'Critic', schema: CRITIC_SCHEMA })
  if (!critic) {
    log(`round ${r}: the critic did not finish; coverage is unknown, so every lens runs again`)
    work = [...lenses, ...retry.filter((l) => !lenses.includes(l))]
  } else {
    work = [...retry, ...critic.gaps.map((g) => ({ key: g.key, title: g.title, rows: g.rows }))]
    if (!work.length) log(`round ${r}: the critic found no further gaps${unswept().length ? `; ${unswept().length} pattern(s) still to sweep` : ''}`)
  }
}
const neverSwept = unswept()
if (neverSwept.length) {
  stopped += `; ${neverSwept.length} pattern(s) were never swept`
  log(`${neverSwept.length} confirmed pattern(s) were never swept: ${neverSwept.join(' ; ')}`)
}
if (round >= MAX_ROUNDS && dry < 2) log(`stopped at the round cap (${MAX_ROUNDS}) before two dry rounds: coverage is NOT complete by the plan's stopping rule`)

// ---------- Live confirmation, one agent so nothing runs against the instance in parallel ----------
const live = confirmed.filter((c) => c.needs_live)
let liveNotes = 'not run'
if (depth === 'full' && live.length) {
  phase('Live')
  const lv = await agent(`${RULES}

Task: the Live phase. Create a disposable local deployment by following the project's own
instructions, and use only that deployment. Reproduce each finding below on it, one at a
time, restoring any setting you change. Then run the fault-injection and abuse tests from
Phase 4 that need a deployed instance and that earlier reviewers listed as not tested:
${notTested.slice(0, 40).join('\n') || '(none listed)'}
Stop the deployment when you finish. Findings to confirm:
${JSON.stringify(live.map((c) => ({ title: c.title, location: c.location, evidence: c.evidence })), null, 1)}`,
    { label: 'live', phase: 'Live', schema: VERDICTS_SCHEMA })
  if (lv) {
    for (const v of lv.verdicts) {
      const hit = confirmed.find((c) => c.title === v.title)
      if (hit) { hit.live_evidence = v.evidence; if (!v.real) hit.live_refuted = true }
      else if (v.real) addConfirmed(v, 'live')
      else refuted.push({ title: v.title, why: `live, unmatched title: ${v.evidence.slice(0, 400)}` })
    }
    liveNotes = `${lv.verdicts.length} checked on a disposable deployment`
  } else liveNotes = 'the live agent did not finish'
} else if (live.length) {
  liveNotes = `${live.length} finding(s) need a deployed instance and were not confirmed live (quick depth)`
}

// ---------- Report ----------
phase('Report')
const order = { critical: 0, high: 1, medium: 2, low: 3 }
confirmed.sort((a, b) => order[a.severity] - order[b.severity])
const report = await agent(`${RULES}

Task: Phase 7 of the plan. Write ${RUN}/REPORT.md with every section the plan lists,
including the critique and the "one step further" section, and merge the grid-*.md files
into its coverage grid. Then update ${REGISTER}: add each confirmed finding under its class
with status open, re-mark past findings you can tell were re-checked, add any new class and
any new pattern. These are the only files you may change.
Run facts: commit ${inv.commit}; depth ${depth}; rounds ${round}; stopped ${stopped}; live phase: ${liveNotes}.
Confirmed findings:
${JSON.stringify(confirmed, null, 1)}
Refuted candidates:
${JSON.stringify(refuted, null, 1)}
Unverified candidates (their verifier did not finish; list them in their own section, and
note any that a later round confirmed or refuted):
${JSON.stringify(unverified, null, 1)}
Patterns never swept for variants (their sweep agent failed or hit the cap; list them as
not swept in the coverage section):
${neverSwept.join('\n') || '(none)'}
Not tested, as reported by reviewers:
${notTested.join('\n') || '(none)'}`,
  { label: 'report', phase: 'Report', schema: REPORT_SCHEMA })

return {
  report: report ? report.report_file : `${RUN} (the report agent did not finish; findings are in this result)`,
  summary: report ? report.summary : '',
  commit: inv.commit,
  depth,
  rounds: round,
  complete_by_stopping_rule: dry >= 2 && !neverSwept.length,
  counts: confirmed.reduce((a, c) => ((a[c.severity] = (a[c.severity] || 0) + 1), a), {}),
  confirmed: confirmed.map((c) => ({ severity: c.severity, title: c.title, location: c.location, fix: c.fix })),
  refuted: refuted.length,
  unverified: unverified.map((f) => ({ severity: f.severity, title: f.title, location: f.location })),
  never_swept: neverSwept,
  live: liveNotes,
}
