// Dry run of .claude/workflows/audit.js with stubbed agents (no model calls).
// Run: node scripts/test-audit-workflow.mjs
import { readFileSync } from 'node:fs'
import assert from 'node:assert/strict'

const src = readFileSync(new URL('../.claude/workflows/audit.js', import.meta.url), 'utf8')
  .replace('export const meta', 'const meta')
const run = Object.getPrototypeOf(async () => {}).constructor(
  'agent', 'pipeline', 'phase', 'log', 'args', src)

const loc = 'src/' + 'x'.repeat(200) + '.rs:1' // 211 chars: longer than the old 160-char key
const empty = { findings: [], grid_file: 'g', not_tested: '' }
const verdict = (title, extra) => ({
  title, real: true, severity: 'medium', evidence: 'e', root_cause: 'same cause', pattern: 'p:' + title,
  fix: 'f', regression_test: 't', needs_live: false, location: loc, ...extra,
})
const logs = []
let reportPrompt = ''
const agent = async (prompt, { label }) => {
  if (label === 'inventory') return {
    run_dir: 'r', commit: 'c', baseline_ok: true, baseline_notes: '',
    lenses: ['A', 'B', 'C'].map((key) => ({ key, title: key, applies: true, touched_since_last_run: true, rows: 'all' })),
  }
  if (label.startsWith('review:A:')) return null // a reviewer that always dies
  if (label === 'review:B:r1') return { ...empty, findings: [{ title: 'b1' }, { title: 'b2' }] }
  if (label === 'review:C:r1') return { ...empty, findings: [{ title: 'c-unverified' }] }
  if (label === 'verify:B:r1') return { verdicts: [verdict('b1', { needs_live: true }), verdict('b2', { location: loc + ' (second site)' })] }
  if (label === 'verify:C:r1') return null // a verifier that dies
  if (label === 'critic:r1') return null
  if (label.startsWith('critic:')) return { gaps: [], assessment: '' }
  if (label === 'live') return { verdicts: [{ ...verdict('b1, retitled'), real: false, evidence: 'not reproducible' }] }
  if (label === 'report') { reportPrompt = prompt; return { report_file: 'r/REPORT.md', summary: '' } }
  return empty
}
const pipeline = async (items, ...stages) => Promise.all(items.map(async (item, i) => {
  let v = item
  for (const stage of stages) {
    try { v = await stage(v, item, i) } catch { return null }
  }
  return v
}))

const res = await run(agent, pipeline, () => {}, (m) => logs.push(m), { depth: 'full' })

assert.equal(res.confirmed.length, 2, 'two findings sharing a 160-char prefix both survive')
assert.ok(logs.some((l) => /critic did not finish/.test(l)), 'a null critic is reported, not "no gaps"')
assert.ok(!logs.some((l) => /round 1: the critic found no further gaps/.test(l)))
assert.ok(logs.some((l) => /returned nothing/.test(l)), 'null agent results are logged')
assert.equal(res.complete_by_stopping_rule, false, 'rounds with a failed agent are not dry')
assert.equal(res.rounds, 5)
assert.ok(res.unverified.some((f) => f.title === 'c-unverified'), 'unverified candidates are returned')
assert.ok(reportPrompt.includes('c-unverified'), 'unverified candidates reach the report')
assert.ok(reportPrompt.includes('live, unmatched title'), 'a retitled live refutation is kept')
console.log('audit workflow dry run: ok')

// A sweep agent that dies: its pattern is swept again next round, even when the critic finds
// no gaps; one that always dies is reported as never swept.
for (const sweepDies of ['once', 'always']) {
  const labels = []
  const logs2 = []
  let prompt2 = ''
  const agent2 = async (prompt, { label }) => {
    labels.push(label)
    if (label === 'inventory') return { run_dir: 'r', commit: 'c', baseline_ok: true, baseline_notes: '',
      lenses: [{ key: 'A', title: 'A', applies: true, touched_since_last_run: true, rows: 'all' }] }
    if (label === 'review:A:r1') return { ...empty, findings: [{ title: 'a1' }] }
    if (label === 'verify:A:r1') return { verdicts: [{ ...verdict('a1'), pattern: 'PAT' }] }
    if (label.startsWith('sweep:')) return sweepDies === 'always' || label === 'sweep:r1:0' ? null : empty
    if (label.startsWith('critic:')) return { gaps: [], assessment: '' }
    if (label === 'report') { prompt2 = prompt; return { report_file: 'x', summary: '' } }
    return empty
  }
  const res2 = await run(agent2, pipeline, () => {}, (m) => logs2.push(m), { depth: 'full' })
  assert.ok(labels.includes('sweep:r2:0'), `${sweepDies}: a failed sweep runs again next round`)
  assert.ok(!logs2.some((l) => /re-queued/.test(l)), `${sweepDies}: no false "re-queued" log`)
  if (sweepDies === 'once') {
    assert.deepEqual(res2.never_swept, [])
  } else {
    assert.deepEqual(res2.never_swept, ['PAT'], 'a pattern whose sweep always fails is reported')
    assert.equal(res2.complete_by_stopping_rule, false)
    assert.ok(/never swept[^]*PAT/.test(prompt2) && /never swept/.test(prompt2.split('Run facts')[1]), 'the report is told')
  }
}
console.log('audit workflow sweep-failure dry run: ok')
