export const meta = {
  name: 'backend-robustness-audit',
  description: 'Resumable, adversarially-verified robustness audit of the Hatchdoor Rust backend (data integrity, concurrency, security). Diverse-lens panel, severity-gated, deterministic rollup.',
  phases: [
    { title: 'Load', detail: 'per-category: read disk checkpoints, decide what still needs doing' },
    { title: 'Find', detail: 'one finder per category, id-stamped & checkpointed', model: 'opus' },
    { title: 'Verify', detail: 'diverse-lens panel (code-truth / failure-injection / already-handled), severity-gated, id-matched', model: 'sonnet' },
    { title: 'Write', detail: 'scribe writes each category report' },
    { title: 'Rollup', detail: 'deterministic in-script SUMMARY built from verified data' },
  ],
}

// ============================================================================
// ============================  JOB CONFIG  ==================================
// Backend robustness audit. Engine below is byte-identical to
// docs/audits/_scaffold/audit-workflow.scaffold.js — only this block differs.
// severityPolicy is deliberately strict: medium is fully verified (3 lenses)
// and even low gets 1 vote, so the finder cannot self-certify a data-loss or
// security bug into the report unchecked.
// ============================================================================
const DEFAULT_CONFIG = {
  jobSlug: 'backend-robustness-audit',
  dir: '/home/battermanz/coding/hatchdoor/docs/audits/backend-robustness',
  repoRoot: '/home/battermanz/coding/hatchdoor',

  domain: {
    subject: 'the Hatchdoor Rust backend (axum HTTP server + SQLite cache + git-sync task + MCP server) in `src/`',
    lens: 'BACKEND robustness — data integrity, concurrency, and security before a public launch',
    // For a backend job "targets" are runtime conditions, not browsers.
    targets: 'a single multi-threaded tokio process in a Linux container serving concurrent HTTP + MCP clients, with a filesystem watcher and an optional git remote. Adverse conditions to reason about: process crash/restart mid-operation, concurrent requests racing shared state, a slow/failing/rejecting git remote, malformed or partially-written vault files, and hostile input (path traversal, oversized bodies, malformed JSON).',
    note: 'The vault on disk (Markdown files) is the source of truth; a SQLite cache mirrors it and is rebuilt/refreshed from it; an optional git-sync task commits and pushes vault changes; an MCP server exposes vault read/write tools. HTTP write handlers, the MCP tools, and the filesystem watcher all mutate shared state concurrently. Assume the process can be killed at any instant.',
    affectedLabel: 'Trigger conditions',
    affectedHint: 'the concrete runtime condition(s) that trigger it (e.g. two concurrent writes, crash between file-write and commit, poisoned RwLock, git push rejected, malformed note path)',
  },

  // Everything is verified (low still gets 1 vote, so nothing is finder-self-certified),
  // but medium drops to a single code-truth lens to save tokens. critical/high get the full panel.
  severityPolicy: { critical: 3, high: 3, medium: 1, low: 1 },

  lenses: [
    {
      name: 'code-truth',
      focus: `LENS — CODE TRUTH. Read the exact cited lines and decide whether the code, as written, actually does what the finding claims. Refute (refuted=true) if: the cited line/symbol is wrong or stale, the claim misreads the control flow or types, the mechanism simply isn't in the code, or a guard/branch/RAII drop already prevents it. You are judging "is the mechanism real in this source?" — nothing else.`,
    },
    {
      name: 'failure-injection',
      focus: `LENS — FAILURE INJECTION. Assume the code is as described; mentally inject the adverse runtime condition and decide whether it actually corrupts data, deadlocks, panics/poisons a lock, leaks a resource, or leaves state inconsistent. Consider: process crash BETWEEN two non-atomic steps (write file then update cache then commit), two requests racing the same file/row, a lock held across an await, a git push rejected or the remote unreachable, a partially-written or malformed file, an error path that leaves a half-applied mutation. Refute (refuted=true) if, under realistic conditions, the bad outcome cannot actually occur (operation is atomic, the lock/ordering prevents the race, the error path rolls back, the fs/SQLite guarantees cover it). You are judging "does the failure actually happen under the stated condition?" — not whether the code reads cleanly.`,
    },
    {
      name: 'already-handled',
      focus: `LENS — ALREADY HANDLED / SEVERITY. Decide whether the case is already mitigated or overstated. Refute (refuted=true) if: a transaction, WAL/atomic-rename, retry, error handler, existing lock, or an upstream crate (rusqlite, git2, tokio, axum) already covers it; another code path makes it unreachable; or the impact is trivial/recoverable so the severity is wrong. You MAY glance at the cited files' imports and neighbouring functions to confirm a guard, but do NOT explore the wider repo.`,
    },
  ],

  models: {
    finder: { model: 'opus', effort: 'medium' },
    panel: { model: 'sonnet', effort: 'medium' },
    scribe: { model: 'haiku', effort: 'low' },
    io: { model: 'haiku', effort: 'low' },
  },

  categories: [
    {
      slug: '01-concurrency-shared-state',
      title: 'Concurrency & shared-state coordination',
      scope: 'AppState shared handles (VaultIndex RwLock, cache access), run_blocking / spawn_blocking usage, the refresh/reindex coordination vs. in-flight reads and writes, the filesystem watcher racing HTTP/MCP mutations, locks held across .await, poisoned-lock handling, and TOCTOU between "check vault" and "act on vault".',
      sources: 'src/app_state.rs, src/vault_watcher.rs, src/vault/index.rs, src/vault.rs, src/cache/mod.rs',
    },
    {
      slug: '02-sqlite-cache-atomicity',
      title: 'SQLite cache atomicity & index integrity',
      scope: 'transaction boundaries and WAL/journal config, partial writes and crash-mid-populate, concurrent read during repopulate/refresh, chunk/embedding row consistency, unique/foreign-key assumptions, busy_timeout / SQLITE_BUSY handling, and whether the cache can diverge from the on-disk vault without detection.',
      sources: 'src/cache/populate.rs, src/cache/queries.rs, src/cache/schema.rs, src/cache/chunk_ops.rs, src/cache/mod.rs',
    },
    {
      slug: '03-git-sync-failure-modes',
      title: 'Git-sync failure modes',
      scope: 'the background sync task: commit/push coalescing and batching, GitError handling, behaviour when the tree is dirty / has conflicts / remote rejects / remote is unreachable / auth fails, any force/reset/stash that could discard user data, retention of unsynced changes across restart, and whether a failed sync can silently drop or clobber vault edits.',
      sources: 'src/git/sync.rs, src/git/task.rs, src/git/status.rs, src/git/config.rs, src/git/mod.rs',
    },
    {
      slug: '04-vault-write-path-safety',
      title: 'Vault write safety & path traversal',
      scope: 'atomicity of note/attachment writes (write-then-rename vs. partial writes), path traversal / absolute-path / symlink escape outside the vault, filename sanitization and collision handling, concurrent writes to the same note, link-rewrite correctness on move/rename, and crash between the file mutation and the cache/git follow-up.',
      sources: 'src/vault/write/notes.rs, src/vault/write/paths.rs, src/vault/write/attachments.rs, src/vault/write/assets.rs, src/vault/write/fs_ops.rs, src/vault/write/rewrites.rs',
    },
    {
      slug: '05-mcp-protocol-surface',
      title: 'MCP protocol & tool-surface robustness',
      scope: 'MCP tool input validation and error shapes, authentication/authorization on the MCP routes, oversized or malformed requests, path/argument validation reaching the same write layer as HTTP, injection or unexpected-state via note content, and consistency of error reporting to MCP clients.',
      sources: 'src/mcp/tools.rs, src/mcp/routes.rs, src/mcp/protocol.rs, src/mcp/config.rs, src/mcp/mod.rs',
    },
    {
      slug: '06-auth-http-handlers',
      title: 'Auth & HTTP handler robustness',
      scope: 'bearer-token auth (comparison, timing, which routes are and are not guarded), body-size limits (the 2 MB axum default vs. advertised limits), download/asset path safety, error handling in write_api, and any route that mutates or leaks data without auth.',
      sources: 'src/auth.rs, src/handlers/api.rs, src/handlers/write_api.rs, src/handlers/downloads.rs, src/handlers/assets.rs, src/main.rs',
    },
    {
      slug: '07-api-error-shape-seam',
      title: 'API error-shape contract seam with the frontend',
      scope: 'the ErrorResponse / status-code shapes the Rust handlers actually return vs. what the frontend assumes in api.ts/writeApi.ts: status codes for auth/validation/not-found/conflict, JSON error body structure, empty-body and non-JSON error cases, and any mismatch that would make the client mis-handle a real backend error.',
      sources: 'src/api_types.rs, src/handlers/api.rs, src/handlers/write_api.rs, frontend/src/api.ts, frontend/src/writeApi.ts',
    },
  ],
}

// ============================================================================
// ==============================  ENGINE  ====================================
// Job-agnostic below this line. Fixes over v1:
//  (1) failure != empty: a finder/panel error throws -> category dropped, NOT
//      marked done, retried on resume (findings.json persists so retry is cheap).
//  (2) id-matched votes: verdicts join findings by stable id, not title string,
//      so a paraphrased title can't silently drop a refutation into "confirmed".
//  (3) deterministic rollup: SUMMARY is built in JS from the verified data the
//      engine already holds — no LLM re-reading files, can't silently no-op.
//  (4) config/engine split (the block above).
//  (5) severityPolicy knob: verification depth per severity is per-job.
//  (6) categories run STRICTLY SEQUENTIALLY (one agent at a time); the finder
//      writes its own findings.json, so an interrupt loses at most the current step.
//  (7) cross-category dedup of confirmed findings in the rollup (by file:line).
//  (8) all durable state is small, engine-owned JSON blobs; the loader validates
//      and treats an unparseable/absent checkpoint as "redo", never as done.
// ============================================================================
const CONFIG = (typeof args !== 'undefined' && args && args.config) || DEFAULT_CONFIG
const DIR = CONFIG.dir
const STATE = `${DIR}/state`
const SEV_RANK = { critical: 0, high: 1, medium: 2, low: 3 }

const votesFor = (sev) => CONFIG.severityPolicy[sev] ?? 0
const killNeeded = (votes) => Math.floor(votes / 2) + 1
const maxLenses = Math.min(CONFIG.lenses.length, Math.max(0, ...Object.values(CONFIG.severityPolicy)))

// ---- schemas ----
const FINDING_PROPS = {
  title: { type: 'string' },
  severity: { type: 'string', enum: ['critical', 'high', 'medium', 'low'] },
  affected: { type: 'array', items: { type: 'string' } },
  file: { type: 'string' },
  line: { type: 'string' },
  description: { type: 'string' },
  why: { type: 'string' },
  fixSketch: { type: 'string' },
}
const FINDINGS_SCHEMA = {
  type: 'object',
  required: ['findings'],
  properties: { findings: { type: 'array', items: { type: 'object', required: Object.keys(FINDING_PROPS), properties: FINDING_PROPS } } },
}
const PANEL_SCHEMA = {
  type: 'object',
  required: ['verdicts'],
  properties: {
    verdicts: {
      type: 'array',
      items: { type: 'object', required: ['id', 'refuted', 'reason'], properties: { id: { type: 'integer' }, refuted: { type: 'boolean' }, reason: { type: 'string' } } },
    },
  },
}
const LOAD_SCHEMA = {
  type: 'object',
  required: ['findings', 'panel', 'verified', 'verifiedPresent', 'reportExists'],
  properties: {
    findings: { type: 'array', items: { type: 'object' } },
    panel: { type: 'array', items: { type: 'object' } }, // [{lens, verdicts:[{id,refuted,reason}]}]
    verified: { type: 'array', items: { type: 'object' } },
    verifiedPresent: { type: 'boolean' }, // did verdicts.json EXIST (even if [])
    reportExists: { type: 'boolean' }, // did the final <slug>.md EXIST and non-empty
  },
}
const OK_SCHEMA = { type: 'object', required: ['ok'], properties: { ok: { type: 'boolean' } } }

// ---- durable-write helper: hand the scribe a finished blob, it copies verbatim ----
async function writeBlob(path, blob, label) {
  await agent(
    `Use the Write tool to write the following EXACT text to ${path}. Copy it byte-for-byte: do not change, summarize, reformat, or comment. If you cannot reproduce it exactly, return {"ok": false}. Otherwise return {"ok": true}.

<<<CONTENT
${blob}
CONTENT`,
    { label, phase: 'Write', ...CONFIG.models.io, schema: OK_SCHEMA },
  )
}

// ---- prompts ----
function finderPrompt(cat) {
  return `You are auditing ${CONFIG.domain.subject} for ${CONFIG.domain.lens}.

NOTE: ${CONFIG.domain.note}

Targets: ${CONFIG.domain.targets}

Category: ${cat.title}
Scope: ${cat.scope}
Primary sources to read (use codegraph_explore and Read; repo root is ${CONFIG.repoRoot}): ${cat.sources}

Find REAL issues that exist in THIS code — not generic lore. Every finding MUST cite an actual file and line you have read. For each, put in \`affected\` ${CONFIG.domain.affectedHint} and explain the concrete reason. Prefer fewer, concrete, code-grounded findings over a long speculative list. If a sub-area is genuinely solid, do not invent issues. Severity reflects launch impact (critical = broken / data-loss / security on a target).

CHECKPOINT: After analysis, use the Write tool to save the findings as {"findings": [...]} to ${STATE}/${cat.slug}.findings.json. Then return the same object via the schema.`
}

function panelPrompt(cat, subset, lensIdx) {
  const lens = CONFIG.lenses[lensIdx]
  const list = subset
    .map((f) => `id=${f.id} [${f.severity}] "${f.title}"\n   at ${f.file}:${f.line}\n   claim: ${f.description}\n   reason given: ${f.why}`)
    .join('\n\n')
  const cited = [...new Set(subset.map((f) => f.file))].join(', ')
  return `You are reviewer (lens: ${lens.name}) on a panel verifying findings for ${CONFIG.domain.subject} (category: ${cat.title}). Repo root: ${CONFIG.repoRoot}.

EFFICIENCY: read ONLY the cited files below — do NOT explore the wider codebase. Cited files: ${cited}. Read each once.

NOTE: ${CONFIG.domain.note}

${lens.focus}

Judge each finding THROUGH YOUR LENS ONLY. Set refuted=true only when your lens exposes a genuine reason the finding is not a real defect on a named target. Set refuted=false if it holds up from your lens — or if your lens is not the right angle to judge it (another reviewer covers that). Do not refute for reasons outside your lens.

Findings to review:
${list}

Return one verdict per finding, keyed by the SAME numeric id shown above: {id, refuted, reason}. Include every id exactly once.`
}

// ---- per-category worker (runs concurrently across categories) ----
async function runCategory(cat) {
  const F = `${STATE}/${cat.slug}.findings.json`
  const P = `${STATE}/${cat.slug}.panel.json`
  const V = `${STATE}/${cat.slug}.verdicts.json`
  const REPORT = `${DIR}/${cat.slug}.md`

  // (1) Load whatever is on disk. Loader distinguishes absent from empty.
  const st = await agent(
    `Read these files if they exist (use: cat <file> 2>/dev/null) and parse each as JSON; also test whether two files exist.
- ${F} -> {"findings":[...]} (array; each may already carry an integer "id")
- ${P} -> array of {"lens":N,"verdicts":[{"id","refuted","reason"}]}
- ${V} -> array of verified finding objects
Return: {"findings": <array or []>, "panel": <array or []>, "verified": <array or []>, "verifiedPresent": <true iff ${V} EXISTS, even if it is []>, "reportExists": <true iff ${REPORT} exists and is non-empty>}. A missing or unparseable file -> [] with its *Present flag false. Do not invent data.`,
    { label: `load:${cat.slug}`, phase: 'Load', ...CONFIG.models.io, schema: LOAD_SCHEMA },
  )

  let findings = (st && st.findings) || []
  const panel = (st && st.panel) || [] // [{lens, verdicts}]
  const verifiedPresent = !!(st && st.verifiedPresent)
  const reportExists = !!(st && st.reportExists)

  // Fully done: report written AND verify checkpoint present. Return loaded data for the rollup.
  if (reportExists && verifiedPresent) {
    log(`✓ ${cat.slug}: already complete — skipping`)
    return { cat, verified: (st && st.verified) || [] }
  }

  // (1/2) Find — only if not already checkpointed. A finder failure THROWS so the
  // category is dropped (null) and retried next resume; it is NOT written as "done".
  if (findings.length === 0 && !verifiedPresent) {
    log(`▶ ${cat.slug}: finding…`)
    const r = await agent(finderPrompt(cat), { label: `find:${cat.slug}`, phase: 'Find', ...CONFIG.models.finder, schema: FINDINGS_SCHEMA })
    if (!r) throw new Error(`finder failed for ${cat.slug}`)
    // The finder already wrote findings.json itself (its CHECKPOINT step). We stamp
    // stable ids in memory here; on resume the loader re-stamps by index identically.
    findings = (r.findings || []).map((f, i) => ({ ...f, id: i })) // (2) stamp stable ids
  } else {
    // ensure ids exist (older checkpoints / resumes)
    findings = findings.map((f, i) => ({ ...f, id: typeof f.id === 'number' ? f.id : i }))
  }

  // (2) Verify — diverse-lens, severity-gated, id-matched. Only if not already done.
  let verified
  if (verifiedPresent) {
    verified = (st && st.verified) || []
  } else if (findings.length === 0) {
    verified = [] // genuine empty finder result — legitimately "nothing found"
  } else {
    const doneLenses = new Set(panel.map((p) => p.lens))
    // lens L reviews every finding whose severity earns > L votes.
    for (let L = 0; L < maxLenses; L++) {
      const subset = findings.filter((f) => votesFor(f.severity) > L)
      if (subset.length === 0 || doneLenses.has(L)) continue
      const r = await agent(panelPrompt(cat, subset, L), { label: `panel:${cat.slug}#${CONFIG.lenses[L].name}`, phase: 'Verify', ...CONFIG.models.panel, schema: PANEL_SCHEMA })
      if (!r) throw new Error(`panel lens ${L} failed for ${cat.slug}`) // findings.json persists -> cheap retry
      panel.push({ lens: L, verdicts: (r.verdicts || []) })
      await writeBlob(P, JSON.stringify(panel, null, 2), `ckpt-panel:${cat.slug}#${L}`)
      log(`  ${cat.slug}: lens ${CONFIG.lenses[L].name} done (${subset.length} findings)`)
    }

    // Aggregate by id.
    const verdictsForId = (id, votes) =>
      panel.filter((p) => p.lens < votes).map((p) => (p.verdicts || []).find((v) => v.id === id)).filter(Boolean)
    verified = findings.map((f) => {
      const votes = votesFor(f.severity)
      if (votes === 0) return { ...f, survives: true, status: 'unverified', refutes: 0, votes: 0, verdicts: [] }
      const vs = verdictsForId(f.id, votes)
      const refutes = vs.filter((v) => v.refuted).length
      const survives = refutes < killNeeded(votes)
      return { ...f, survives, status: 'verified', refutes, votes: vs.length, expectedVotes: votes, incompleteVotes: vs.length < votes, verdicts: vs }
    })
    await writeBlob(V, JSON.stringify(verified, null, 2), `ckpt-verdicts:${cat.slug}`)
  }

  // (3) Report — always (re)written from verified; idempotent.
  const confirmed = verified.filter((v) => v.survives)
  const rejected = verified.filter((v) => !v.survives)
  await agent(
    `Write a markdown audit report to ${REPORT} using the Write tool. Category title: "${cat.title}".

Structure:
- H1 with the category title.
- One-line summary: N confirmed (by severity), M refuted. Note LOW/unverified findings were not voted on.
- H2 "Confirmed findings": for EACH confirmed finding an H3 "SEVERITY: title" (append " (unverified)" if status is unverified, " (incomplete votes)" if incompleteVotes is true), then bullets: ${CONFIG.domain.affectedLabel} (join \`affected\`), Location (\`file:line\`), What happens, Why, Fix sketch. Order critical > high > medium > low.
- H2 "Refuted (not real / already handled)": each rejected finding title + one-line reason drawn from its verdicts.

Data (JSON):
${JSON.stringify({ confirmed, rejected }, null, 2)}

After writing, return {"ok": true}.`,
    { label: `write:${cat.slug}`, phase: 'Write', ...CONFIG.models.scribe, schema: OK_SCHEMA },
  )
  log(`✔ ${cat.slug}: banked (${confirmed.length} confirmed, ${rejected.length} refuted)`)
  return { cat, verified }
}

// ---- deterministic rollup (fix 3 + 7): built in JS from verified data ----
function buildSummary(results) {
  const rows = []
  for (const { cat, verified } of results) {
    for (const f of verified.filter((v) => v.survives)) {
      rows.push({
        sev: f.severity,
        rank: SEV_RANK[f.severity] ?? 9,
        cat: cat.slug,
        title: f.title,
        affected: (f.affected || []).join(', '),
        loc: `${f.file}:${f.line}`,
        unverified: f.status === 'unverified',
        incomplete: !!f.incompleteVotes,
        key: `${(f.file || '').trim().toLowerCase()}:${(f.line || '').toString().trim()}`,
      })
    }
  }
  // (7) dedup by file:line — collapse cross-category duplicates, note co-flagging categories.
  const byKey = new Map()
  for (const r of rows) {
    const ex = byKey.get(r.key)
    if (!ex) byKey.set(r.key, { ...r, cats: [r.cat] })
    else {
      if (!ex.cats.includes(r.cat)) ex.cats.push(r.cat)
      if (r.rank < ex.rank) { ex.rank = r.rank; ex.sev = r.sev; ex.title = r.title }
    }
  }
  const deduped = [...byKey.values()].sort((a, b) => a.rank - b.rank || a.cats[0].localeCompare(b.cats[0]))
  const tag = (r) => (r.unverified ? ' _(unverified)_' : '') + (r.incomplete ? ' _(incomplete votes)_' : '') + (r.cats.length > 1 ? ` _(also: ${r.cats.slice(1).join(', ')})_` : '')

  const counts = deduped.reduce((m, r) => ((m[r.sev] = (m[r.sev] || 0) + 1), m), {})
  const blockers = deduped.filter((r) => r.sev === 'critical' || r.sev === 'high')

  let md = `# ${CONFIG.jobSlug} — rollup\n\n`
  md += `Audit of ${CONFIG.domain.subject} (${CONFIG.domain.lens}).\n\n`
  md += `**Confirmed (deduped): ${deduped.length} — ` +
    ['critical', 'high', 'medium', 'low'].filter((s) => counts[s]).map((s) => `${counts[s]} ${s}`).join(' · ') + `.**\n\n`

  md += `## Top blockers (critical / high)\n\n`
  if (blockers.length === 0) md += `_None._\n\n`
  else {
    md += `| Sev | Category | Title | ${CONFIG.domain.affectedLabel} | Location |\n|---|---|---|---|---|\n`
    for (const r of blockers) md += `| ${r.sev.toUpperCase()} | ${r.cats.join(', ')} | ${r.title}${tag(r)} | ${r.affected} | \`${r.loc}\` |\n`
    md += `\n`
  }

  md += `## All confirmed findings\n\n`
  md += `| Sev | Category | Title | ${CONFIG.domain.affectedLabel} | Location |\n|---|---|---|---|---|\n`
  for (const r of deduped) md += `| ${r.sev.toUpperCase()} | ${r.cats.join(', ')} | ${r.title}${tag(r)} | ${r.affected} | \`${r.loc}\` |\n`
  md += `\n_Per-category detail (what happens / why / fix) is in each \`NN-*.md\`. Rows tagged "unverified" are finder-graded low severity not voted on; "incomplete votes" means a panel lens did not return a verdict for that finding._\n`
  return md
}

// ============================  DRIVER  ======================================
log(`${CONFIG.jobSlug}: ${CONFIG.categories.length} categories; policy ${JSON.stringify(CONFIG.severityPolicy)}; ${maxLenses} max lenses`)

// Categories run STRICTLY SEQUENTIALLY — one agent at a time, one category fully
// (find → checkpoint → verify → checkpoint → report) before the next starts. A
// category whose finder or panel throws is dropped and retried on resume; its
// findings.json (written by the finder itself) persists so the retry is cheap.
const results = []
for (const cat of CONFIG.categories) {
  try {
    results.push(await runCategory(cat))
  } catch (e) {
    log(`⚠ ${cat.slug} failed this run (${e.message}) — will resume next run; finder work is cached.`)
  }
}
const failed = CONFIG.categories.length - results.length
if (failed > 0) log(`⚠ ${failed} category(ies) did not complete this run — rerun to resume them (finder work is cached).`)

phase('Rollup')
if (results.length > 0) {
  await writeBlob(`${DIR}/SUMMARY.md`, buildSummary(results), 'rollup:SUMMARY')
  log(`SUMMARY.md written from ${results.length} categories`)
}

return { done: results.map((r) => r.cat.slug), failed }
