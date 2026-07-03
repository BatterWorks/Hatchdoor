export const meta = {
  name: 'audit-fix-implementer',
  description: 'Resumable, verification-gated implementer of confirmed client-edge-case audit findings. Works in a scratch worktree, one commit per fix that passes the full build+test gate, reverts+flags any that fail. The driver fast-forwards passing commits onto development.',
  phases: [
    { title: 'Setup', detail: 'clean the worktree, collect confirmed high+medium findings, load the ledger' },
    { title: 'Fix', detail: 'per finding: test-first (red) where testable, implement, gate (build+test+lint), commit or revert+flag', model: 'opus' },
    { title: 'Rollup', detail: 'deterministic FIXES.md + done sentinel from the ledger' },
  ],
}

// ============================================================================
// ============================  JOB CONFIG  ==================================
// Implements the CONFIRMED findings from the two audits. Runs unattended across
// usage windows via run-fix-driver.sh. Every source change is gated by the real
// build+test suite in an isolated worktree; only a green change is committed.
// A change that can't be made green is reverted and flagged for a human.
// ============================================================================
const CONFIG = {
  repoRoot: '/home/battermanz/coding/hatchdoor',
  worktree: '/home/battermanz/coding/hatchdoor-audit-fixes', // created + deps-linked by the driver
  branch: 'audit-fixes',
  dir: '/home/battermanz/coding/hatchdoor/docs/audits/_fixes',

  // Only these severities are attempted. Lows are finder-unverified — included by
  // user decision; they still carry survives===true in the client verdicts files.
  severities: ['high', 'medium', 'low'],

  // Up to this many repair attempts after a failing gate before reverting+flagging.
  repairAttempts: 1,

  // Hybrid TDD: for each finding, first try to write a failing (red) test that
  // reproduces it; if a deterministic repro isn't feasible, fall back to the
  // regression gate. Each fix is tagged with the mode that proved it.
  tddWhereTestable: true,

  // Where the confirmed findings live. Each glob yields <slug>.verdicts.json arrays.
  // Backend audit is already fixed (on development) — client-edge-cases only.
  auditStateGlobs: [
    '/home/battermanz/coding/hatchdoor/docs/audits/client-edge-cases/state',
  ],

  // Verification gates, run INSIDE the worktree. Language auto-detected from the
  // changed files. Formatters run first (auto-fix, never a failure reason), then
  // the hard checks. Any non-zero check = gate fail.
  gates: {
    rust: {
      match: (f) => f.endsWith('.rs') || f === 'Cargo.toml',
      cwd: '.',
      format: ['cargo fmt'],
      checks: ['cargo build --locked', 'cargo test --locked', 'cargo clippy --all-targets --locked -- -D warnings'],
    },
    frontend: {
      match: (f) => f.startsWith('frontend/'),
      cwd: 'frontend',
      format: ['npx prettier --write .', 'npx eslint . --fix'],
      checks: ['npm run typecheck', 'npm run test', 'npm run lint', 'npm run build'],
    },
  },

  models: {
    fix: { model: 'opus', effort: 'high' },
    gate: { model: 'sonnet', effort: 'medium' },
    io: { model: 'haiku', effort: 'low' },
  },
}

const WT = CONFIG.worktree
const STATE = `${CONFIG.dir}/state`
const LEDGER = `${STATE}/ledger.json`
const FIXES = `${CONFIG.dir}/FIXES.md`
const DONE = `${STATE}/.fixes-complete`
const SEV_RANK = { critical: 0, high: 1, medium: 2, low: 3 }
const CO_AUTHOR = 'Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>'
const KEY_TRAILER = 'Audit-Fix-Key'

// ---- schemas ----
const SH_SCHEMA = { type: 'object', required: ['code', 'out'], properties: { code: { type: 'integer' }, out: { type: 'string' } } }
const OK_SCHEMA = { type: 'object', required: ['ok'], properties: { ok: { type: 'boolean' } } }
const FINDINGS_SCHEMA = {
  type: 'object',
  required: ['findings'],
  properties: {
    findings: {
      type: 'array',
      items: {
        type: 'object',
        required: ['key', 'audit', 'category', 'severity', 'title', 'file', 'line', 'description', 'why', 'fixSketch', 'affected'],
        properties: {
          key: { type: 'string' }, audit: { type: 'string' }, category: { type: 'string' },
          severity: { type: 'string' }, title: { type: 'string' }, file: { type: 'string' }, line: { type: 'string' },
          description: { type: 'string' }, why: { type: 'string' }, fixSketch: { type: 'string' },
          affected: { type: 'array', items: { type: 'string' } },
        },
      },
    },
  },
}
const LEDGER_SCHEMA = {
  type: 'object',
  required: ['done', 'failed', 'gitKeys'],
  properties: {
    done: { type: 'array', items: { type: 'object' } },
    failed: { type: 'array', items: { type: 'object' } },
    gitKeys: { type: 'array', items: { type: 'string' } }, // keys already committed (from git log trailers)
  },
}
const FIX_SCHEMA = {
  type: 'object',
  required: ['filesChanged', 'summary'],
  properties: { filesChanged: { type: 'array', items: { type: 'string' } }, summary: { type: 'string' }, abstained: { type: 'boolean' } },
}
const GATE_SCHEMA = {
  type: 'object',
  required: ['pass', 'ranLangs', 'failLog'],
  properties: { pass: { type: 'boolean' }, ranLangs: { type: 'array', items: { type: 'string' } }, failingCheck: { type: 'string' }, failLog: { type: 'string' } },
}
const COMMIT_SCHEMA = { type: 'object', required: ['committed', 'hash'], properties: { committed: { type: 'boolean' }, hash: { type: 'string' } } }
const TDD_SCHEMA = {
  type: 'object',
  required: ['testable', 'redConfirmed', 'testFiles', 'reason'],
  properties: { testable: { type: 'boolean' }, redConfirmed: { type: 'boolean' }, testFiles: { type: 'array', items: { type: 'string' } }, reason: { type: 'string' } },
}

// ---- helpers ----
async function sh(cmd, label) {
  const r = await agent(
    `Run exactly this shell command and report its result. Do not add flags, do not run anything else.\n\nCOMMAND:\n${cmd}\n\nReturn {"code": <exit status integer>, "out": "<combined stdout+stderr, last ~4000 chars>"}.`,
    { label, phase: 'Setup', ...CONFIG.models.io, schema: SH_SCHEMA },
  )
  return r || { code: 1, out: 'agent-null' }
}

async function writeBlob(path, blob, label, phase = 'Rollup') {
  await agent(
    `Use the Write tool to write the following EXACT text to ${path}, byte-for-byte. Do not change, summarize, or comment. Return {"ok": true} on success, else {"ok": false}.\n\n<<<CONTENT\n${blob}\nCONTENT`,
    { label, phase, ...CONFIG.models.io, schema: OK_SCHEMA },
  )
}

const gatesForFiles = (files) => Object.entries(CONFIG.gates).filter(([, g]) => files.some((f) => g.match(f))).map(([name]) => name)

// ---- prompts ----
const rustTestCheck = CONFIG.gates.rust.checks.find((c) => c.includes('test')) || 'cargo test --locked'

function testFirstPrompt(f) {
  const src = `${WT}/${f.file}`
  return `You are doing TEST-FIRST (the RED step) for ONE confirmed audit finding, in the isolated git worktree ${WT}. Do NOT fix the bug in this step. All paths are under ${WT}; never touch ${CONFIG.repoRoot}.

FINDING [${f.severity}] — ${f.title}
File: ${src} (cited lines ${f.line})
What happens: ${f.description}
Why it matters: ${f.why}

Step 1 — JUDGE TESTABILITY. A finding is testable ONLY if you can write a DETERMINISTIC automated test that FAILS on the current (unfixed) code and would pass once fixed. Perf/battery/visual-layout/frame-rate properties and nondeterministic timing/concurrency races are NOT deterministically testable. If it is not cleanly testable, write NOTHING and return {"testable": false, "redConfirmed": false, "testFiles": [], "reason": "<why not>"}.

Step 2 — WRITE THE RED TEST. If testable, write a minimal test that reproduces the finding, placed where the EXISTING runner already discovers it (Rust: a #[test]/#[tokio::test] in the relevant module or its tests file; frontend: a *.test.ts(x) under frontend/src matching the current vitest globs). Do NOT modify any production/non-test code.

Step 3 — CONFIRM RED. Run the real suite for that language (Rust: \`cd ${WT} && ${rustTestCheck}\`; frontend: \`cd ${WT}/frontend && npm run test\`) and check YOUR new test fails for the finding's reason.
- Fails as expected -> return {"testable": true, "redConfirmed": true, "testFiles": ["<paths>"], "reason": "reproduced"}.
- Unexpectedly PASSES on unfixed code (cannot reproduce): DELETE your test (leave the tree clean) and return {"testable": true, "redConfirmed": false, "testFiles": [], "reason": "no red repro: <detail>"}.

Return only the JSON. Do NOT commit.`
}

function fixPrompt(f, mode) {
  const src = `${WT}/${f.file}`
  const tddNote = mode === 'tdd'
    ? `\nA failing test that reproduces this finding has ALREADY been written and currently fails (RED). Implement the PRODUCTION fix so that test — and the whole suite — passes. Do NOT edit, weaken, or delete the test to force a pass.\n`
    : ''
  return `You are implementing ONE confirmed audit fix in an isolated git worktree at ${WT}. All edits MUST be made inside that worktree (absolute paths under ${WT}); never touch ${CONFIG.repoRoot}.

FINDING [${f.severity}] — ${f.title}
Primary file: ${src} (cited lines ${f.line})
What happens: ${f.description}
Why it matters: ${f.why}
Suggested fix: ${f.fixSketch}
Affected: ${(f.affected || []).join(', ')}
${tddNote}
Instructions:
- Read the cited file (and only what you need around it) IN THE WORKTREE, then apply the MINIMAL correct fix. Prefer the suggested fix but implement what is actually correct for the code as written.
- Keep the change tightly scoped to this finding. Do not refactor unrelated code, reformat files, or fix other findings.${mode === 'tdd' ? ' Do NOT touch the already-written test file(s).' : ''}
- Do NOT run build/test/git — the engine gates and commits separately.
- If, after reading the real code, this finding is stale/wrong/already-fixed and no change is warranted, make NO edits and return {"filesChanged": [], "summary": "why no change", "abstained": true}.

Return {"filesChanged": ["<repo-relative paths you edited>"], "summary": "<one line>", "abstained": <bool>}.`
}

function gatePrompt(f) {
  return `You are the verification gate for an audit fix in the worktree ${WT}. Repo languages: Rust (root Cargo) + a Vite/TS frontend in frontend/.

1. Determine changed files: run \`git -C ${WT} status --porcelain\`. Map to languages: any *.rs or Cargo.toml => rust; any frontend/* => frontend.
2. For EACH detected language, cd into the worktree and run, IN ORDER, stopping at the first failure:
   - rust (cwd ${WT}): ${CONFIG.gates.rust.format.join(' ; ')} ; then ${CONFIG.gates.rust.checks.join(' ; ')}
   - frontend (cwd ${WT}/frontend): ${CONFIG.gates.frontend.format.join(' ; ')} ; then ${CONFIG.gates.frontend.checks.join(' ; ')}
   The formatters auto-fix style and are NOT failures. A non-zero exit on any CHECK is a gate failure.
3. If nothing changed (empty status), that is a PASS with ranLangs [] (the fix abstained).

Return {"pass": <true iff every check of every detected language exited 0>, "ranLangs": ["rust"/"frontend"...], "failingCheck": "<the command that failed, if any>", "failLog": "<last ~3000 chars of the failing command's output, else empty>"}.`
}

function repairPrompt(f, failingCheck, failLog) {
  return `Your previous fix for "${f.title}" FAILED the gate in worktree ${WT}. Failing check: ${failingCheck}\n\nOutput (tail):\n${failLog}\n\nRepair the PRODUCTION change IN THE WORKTREE so this check passes, keeping the fix's intent and staying minimal. Never edit or delete a test file to force a pass. Do not run build/test/git; the engine re-gates. If it cannot be fixed cleanly, revert your production edits (leave the code as upstream) and return abstained. Return {"filesChanged":[...], "summary":"...", "abstained": <bool>}.`
}

function commitPrompt(f) {
  const msg = `fix(audit): ${f.title.slice(0, 68)}\n\nAddresses confirmed ${f.severity} finding ${f.key} (${f.category}).\n${f.file}:${f.line}\n\n${KEY_TRAILER}: ${f.key}\n${CO_AUTHOR}`
  return `In the worktree ${WT}, stage and commit the current fix, then report the hash.\nRun:\n  git -C ${WT} add -A\n  git -C ${WT} commit -F - <<'MSG'\n${msg}\nMSG\n  git -C ${WT} rev-parse --short HEAD\nReturn {"committed": true, "hash": "<short hash from the last command>"}. If there was nothing to commit, return {"committed": false, "hash": ""}.`
}

// ============================  DRIVER  ======================================
phase('Setup')

// Guard: the client-edge-case audit must be complete (its SUMMARY.md exists).
// The shell driver also checks this, but double-check so a manual run can't jump the gun.
const ready = await sh(
  `test -s ${CONFIG.repoRoot}/docs/audits/client-edge-cases/SUMMARY.md && echo READY || echo WAIT`,
  'guard:audit-complete',
)
if (!/READY/.test(ready.out)) {
  log('Client audit not complete yet (missing SUMMARY.md) — nothing to fix. Exiting.')
  return { waiting: true }
}

// Clean slate: discard any uncommitted edits from a tick that died mid-fix.
// Committed fixes are preserved; only in-flight junk is cleared.
await sh(`git -C ${WT} reset --hard && git -C ${WT} clean -fd`, 'worktree:clean')

// Collect confirmed high+medium findings across both audits, with stable keys.
const collected = await agent(
  `Collect confirmed audit findings. For each state dir below, list its *.verdicts.json files and read each (they are JSON arrays):\n${CONFIG.auditStateGlobs.map((g) => `- ${g}`).join('\n')}\n\nFor every array element where \`survives\` === true AND severity is one of ${JSON.stringify(CONFIG.severities)}, emit one finding:\n- audit  = the audit folder name (the path segment before "/state")\n- category = the verdicts filename without ".verdicts.json"\n- index  = its 0-based position in that file's array\n- key    = "<audit>/<category>#<index>"  (STABLE — do not change the scheme)\n- severity, title, file, line, description, why, fixSketch  (copy verbatim)\n- affected = the element's \`affected\` array, or \`affectedClients\`, or []\nSkip elements without survives===true. Do not invent findings. Return {"findings":[...]}.`,
  { label: 'collect:findings', phase: 'Setup', ...CONFIG.models.io, schema: FINDINGS_SCHEMA },
)
const findings = (collected && collected.findings) || []
findings.sort((a, b) => (SEV_RANK[a.severity] ?? 9) - (SEV_RANK[b.severity] ?? 9) || a.key.localeCompare(b.key))

// Load ledger (JSON) + reconstruct already-committed keys from git-log trailers
// (covers a commit that landed before its ledger write on a killed tick).
const led = await agent(
  `Report the current fix state:\n1. Read ${LEDGER} if it exists (JSON {"done":[...],"failed":[...]}); missing => both [].\n2. List keys already committed on the branch: run \`git -C ${WT} log --format=%B | grep -oE '${KEY_TRAILER}: .+' | sed 's/${KEY_TRAILER}: //'\` and return them as gitKeys (dedup; [] if none).\nReturn {"done": <array>, "failed": <array>, "gitKeys": <array of strings>}.`,
  { label: 'load:ledger', phase: 'Setup', ...CONFIG.models.io, schema: LEDGER_SCHEMA },
)
const ledger = { done: (led && led.done) || [], failed: (led && led.failed) || [] }
const committedKeys = new Set([...(led && led.gitKeys || []), ...ledger.done.map((d) => d.key)])
// Reconcile: any git-committed key missing from ledger.done gets recorded.
for (const k of committedKeys) if (!ledger.done.find((d) => d.key === k)) ledger.done.push({ key: k, title: '(recovered from git log)', reconciled: true })

const terminal = new Set([...ledger.done.map((d) => d.key), ...ledger.failed.map((f) => f.key)])
const pending = findings.filter((f) => !terminal.has(f.key))
log(`${findings.length} confirmed ${CONFIG.severities.join('/')} findings · ${ledger.done.length} done · ${ledger.failed.length} flagged · ${pending.length} pending`)

// Fix loop — STRICTLY SEQUENTIAL (one branch, stateful build gate).
phase('Fix')
for (const f of pending) {
  log(`▶ ${f.key} [${f.severity}] ${f.title.slice(0, 70)}`)

  // (1) Hybrid TDD — try to plant a failing (red) test that reproduces the finding.
  // mode 'tdd' => a real red test now guards this fix; 'regression' => suite-only.
  let mode = 'regression'
  if (CONFIG.tddWhereTestable) {
    const t = await agent(testFirstPrompt(f), { label: `test:${f.key}`, phase: 'Fix', ...CONFIG.models.fix, schema: TDD_SCHEMA })
    if (t && t.testable && t.redConfirmed) {
      mode = 'tdd'
      log(`  ● ${f.key}: red test written (${(t.testFiles || []).join(', ') || 'test'})`)
    } else if (t && t.testable && !t.redConfirmed) {
      // couldn't produce a red repro — discard any leftover test artifacts, fall back.
      await sh(`git -C ${WT} reset --hard && git -C ${WT} clean -fd`, `test-discard:${f.key}`)
      log(`  · ${f.key}: no red repro (${t.reason || ''}) — regression-gated`)
    } else {
      log(`  · ${f.key}: not deterministically testable — regression-gated`)
    }
  }

  // (2) Implement the production fix (test file, if any, stays untouched).
  const fx = await agent(fixPrompt(f, mode), { label: `fix:${f.key}`, phase: 'Fix', ...CONFIG.models.fix, schema: FIX_SCHEMA })

  if (fx && fx.abstained && (!fx.filesChanged || fx.filesChanged.length === 0)) {
    await sh(`git -C ${WT} reset --hard && git -C ${WT} clean -fd`, `abstain-clean:${f.key}`)
    ledger.failed.push({ key: f.key, title: f.title, severity: f.severity, mode, reason: `abstained: ${fx.summary || 'no change warranted'}`, abstained: true })
    await writeBlob(LEDGER, JSON.stringify(ledger, null, 2), `ckpt:${f.key}`, 'Fix')
    log(`  ↷ ${f.key}: abstained — ${fx.summary || ''}`)
    continue
  }

  // (3) Gate, with bounded repair attempts. In tdd mode the planted test is part
  // of the suite the gate runs, so a green gate is a genuine red->green transition.
  let g = await agent(gatePrompt(f), { label: `gate:${f.key}`, phase: 'Fix', ...CONFIG.models.gate, schema: GATE_SCHEMA })
  let attempt = 0
  while ((!g || !g.pass) && attempt < CONFIG.repairAttempts) {
    attempt++
    log(`  ⚠ ${f.key}: gate failed (${g && g.failingCheck || '?'}) — repair ${attempt}/${CONFIG.repairAttempts}`)
    await agent(repairPrompt(f, g && g.failingCheck || '', g && g.failLog || ''), { label: `repair:${f.key}#${attempt}`, phase: 'Fix', ...CONFIG.models.fix, schema: FIX_SCHEMA })
    g = await agent(gatePrompt(f), { label: `gate:${f.key}#${attempt}`, phase: 'Fix', ...CONFIG.models.gate, schema: GATE_SCHEMA })
  }

  if (g && g.pass) {
    const c = await agent(commitPrompt(f), { label: `commit:${f.key}`, phase: 'Fix', ...CONFIG.models.io, schema: COMMIT_SCHEMA })
    if (c && c.committed) {
      ledger.done.push({ key: f.key, title: f.title, severity: f.severity, file: f.file, langs: g.ranLangs, mode, hash: c.hash })
      log(`  ✔ ${f.key}: committed ${c.hash} [${mode}] (${(g.ranLangs || []).join('+') || 'no-op'})`)
    } else {
      // Passed gate but nothing to commit (e.g. formatter-only / abstain slipped through) — flag, don't loop forever.
      await sh(`git -C ${WT} reset --hard && git -C ${WT} clean -fd`, `nocommit-clean:${f.key}`)
      ledger.failed.push({ key: f.key, title: f.title, severity: f.severity, mode, reason: 'gate passed but no committable change produced' })
      log(`  ↷ ${f.key}: no committable change`)
    }
  } else {
    // Give up: revert this fix (and its test) entirely, flag for a human, move on.
    await sh(`git -C ${WT} reset --hard && git -C ${WT} clean -fd`, `revert:${f.key}`)
    ledger.failed.push({ key: f.key, title: f.title, severity: f.severity, mode, reason: `gate failed: ${g && g.failingCheck || 'unknown'}`, failLog: (g && g.failLog || '').slice(-1200) })
    log(`  ✘ ${f.key}: reverted + flagged (${g && g.failingCheck || 'gate failure'})`)
  }
  await writeBlob(LEDGER, JSON.stringify(ledger, null, 2), `ckpt:${f.key}`, 'Fix')
}

// ---- deterministic rollup ----
phase('Rollup')
function buildFixesReport() {
  const done = [...ledger.done].filter((d) => !d.reconciled || findings.find((f) => f.key === d.key))
  done.sort((a, b) => (SEV_RANK[a.severity] ?? 9) - (SEV_RANK[b.severity] ?? 9))
  const failed = [...ledger.failed].sort((a, b) => (SEV_RANK[a.severity] ?? 9) - (SEV_RANK[b.severity] ?? 9))
  let md = `# Audit fixes — implementation log\n\n`
  md += `Automated implementation of confirmed **${CONFIG.severities.join(' / ')}** findings from the client-edge-case audit. Where a finding was deterministically testable, a failing test was written first (red) and the fix made it pass (green); otherwise the fix was regression-gated. Every committed fix passed the full build + test gate in a scratch worktree, then the driver fast-forwarded it onto \`development\`. Each carries an \`${KEY_TRAILER}\` trailer. Review the \`regr\` rows by hand.\n\n`
  md += `**${done.length} committed · ${failed.length} flagged for human · ${pending.length === 0 ? 'all findings processed' : pending.length + ' still pending'}.**\n\n`
  const tddCount = done.filter((d) => d.mode === 'tdd').length
  md += `## Committed fixes\n\n`
  if (!done.length) md += `_None yet._\n\n`
  else {
    md += `${tddCount} proven by a new failing→passing test (\`tdd\`); ${done.length - tddCount} regression-gated only (\`regr\` — no deterministic test was feasible; verify behaviour by hand).\n\n`
    md += `| Sev | Proof | Finding | Location | Gate | Commit |\n|---|---|---|---|---|---|\n`
    for (const d of done) md += `| ${(d.severity || '').toUpperCase()} | ${d.mode === 'tdd' ? 'tdd' : 'regr'} | ${d.title || d.key} | \`${d.file || ''}\` | ${(d.langs || []).join('+') || '—'} | \`${d.hash || '—'}\` |\n`
    md += `\n`
  }
  md += `## Flagged for human (not fixed)\n\n`
  if (!failed.length) md += `_None._\n\n`
  else {
    md += `| Sev | Finding | Reason |\n|---|---|---|\n`
    for (const f of failed) md += `| ${(f.severity || '').toUpperCase()} | ${f.title || f.key} | ${(f.reason || '').replace(/\|/g, '\\|').slice(0, 200)} |\n`
    md += `\n_Full failing-gate output for each is in \`state/ledger.json\`._\n`
  }
  return md
}
await writeBlob(FIXES, buildFixesReport(), 'rollup:FIXES')

if (pending.length === 0) {
  await sh(`touch ${DONE}`, 'sentinel:done')
  log(`All findings processed — ${ledger.done.length} committed on \`${CONFIG.branch}\`, ${ledger.failed.length} flagged. Wrote ${DONE}.`)
}
return { committed: ledger.done.length, flagged: ledger.failed.length, pending: pending.length }
