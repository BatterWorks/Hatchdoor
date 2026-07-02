export const meta = {
  name: 'audit-scaffold',
  description: 'Reusable, resumable, adversarially-verified audit engine: finder -> diverse-lens severity-gated panel -> scribe -> deterministic rollup. Swap the CONFIG block per job.',
  phases: [
    { title: 'Load', detail: 'per-category: read disk checkpoints, decide what still needs doing' },
    { title: 'Find', detail: 'one finder per category, id-stamped & checkpointed', model: 'opus' },
    { title: 'Verify', detail: 'diverse-lens panel, severity-gated, id-matched votes', model: 'sonnet' },
    { title: 'Write', detail: 'scribe writes each category report' },
    { title: 'Rollup', detail: 'deterministic in-script SUMMARY built from verified data' },
  ],
}

// ============================================================================
// ============================  JOB CONFIG  ==================================
// Swap THIS block per job. The engine below is job-agnostic. You can also pass
// an override at launch via args.config (same shape) without editing the file.
// The default below reproduces the client edge-case audit as a worked example;
// `dir` points at a sandbox so an accidental run never clobbers a real audit.
// ============================================================================
const DEFAULT_CONFIG = {
  jobSlug: 'client-edge-case-audit',
  dir: '/home/battermanz/coding/hatchdoor/docs/audits/_scaffold/example-run',
  repoRoot: '/home/battermanz/coding/hatchdoor',

  // Framing woven into every finder/panelist prompt.
  domain: {
    subject: 'the Hatchdoor React 19 + TypeScript + Vite PWA frontend',
    lens: 'CLIENT/BROWSER edge cases before a public launch',
    targets: 'Desktop Chrome, Edge, Firefox, desktop Safari (WebKit), iOS Safari + installed PWA (WebKit), Android Chrome',
    note: 'the app has BOTH a read-only vault viewer AND a write mode (create/edit/delete notes, image upload, drafts). Do not assume it is read-only.',
    // Label for the per-finding "who/what is affected" field (clients here; could be "trigger conditions" for a backend job).
    affectedLabel: 'Affected clients',
    // What the finder should put in `affected`.
    affectedHint: 'the specific target client(s)/engine(s) that misbehave',
  },

  // votes-per-severity: how many DISTINCT lenses (from `lenses`, in order) review a
  // finding of each severity. 0 = unverified (surfaced but not voted on). A finding
  // is refuted when refutes >= floor(votes/2)+1 (majority; 1 vote => 1 kills).
  // For a data-loss/security job, raise medium/low so the finder can't self-certify.
  severityPolicy: { critical: 3, high: 3, medium: 1, low: 0 },

  // Independent lenses, applied in order. lenses[0] is the primary (used for the
  // lower, single-vote tiers). Keep them genuinely different angles.
  lenses: [
    {
      name: 'code-truth',
      focus: `LENS — CODE TRUTH. Read the exact cited lines and decide whether the code, as written, actually does what the finding claims. Refute (refuted=true) if: the cited line/symbol is wrong or stale, the claim misreads the control flow, the mechanism simply isn't in the code, or an in-function guard/branch already prevents it. You are judging "is the mechanism real in this source?" — nothing else.`,
    },
    {
      name: 'device-repro',
      focus: `LENS — DEVICE REPRO. Assume the code is as described; decide whether it actually reproduces on the NAMED target. Refute (refuted=true) if: the named engine/environment does not behave as claimed, the capability is actually present on all named targets, the affected list is wrong, or the trigger path can't occur for a real user. You are judging "does this bite a real user on the stated target?" — not whether the code is correct.`,
    },
    {
      name: 'already-handled',
      focus: `LENS — ALREADY HANDLED / SEVERITY. Decide whether the case is already mitigated or overstated. Refute (refuted=true) if: a fallback, guard, error handler, or upstream dependency already covers it, another code path makes it unreachable, or the impact is trivial/recoverable so the severity is wrong. You MAY glance at the cited files' imports to confirm a guard, but do NOT explore the wider repo.`,
    },
  ],

  // Model/effort per role — one place to retune cost vs. depth.
  models: {
    finder: { model: 'opus', effort: 'medium' },
    panel: { model: 'sonnet', effort: 'medium' },
    scribe: { model: 'haiku', effort: 'low' },
    io: { model: 'haiku', effort: 'low' },
  },

  categories: [
    {
      slug: '01-service-worker-pwa',
      title: 'Service worker / PWA / offline / install / cache staleness',
      scope: 'Workbox service worker (vite-plugin-pwa, registerType autoUpdate), the registerSW onNeedRefresh that calls window.location.reload(), navigateFallback + denylist behaviour, offline behaviour, cache staleness across deploys, install/standalone lifecycle, update races that could reload mid-edit and lose unsaved draft state.',
      sources: 'frontend/vite.config.ts, frontend/src/main.tsx, frontend/src/writeDrafts.ts, frontend/src/app/storage.ts',
    },
    {
      slug: '02-clipboard-upload-download',
      title: 'Clipboard / image upload / file download',
      scope: 'navigator.clipboard availability + permissions across engines, copy fallbacks, image paste/upload, the file-download path (download attribute ignored on iOS standalone PWA), blob URL lifetime, MIME handling.',
      sources: 'frontend/src/clipboard.ts, frontend/src/imageUpload.ts, frontend/src/App.links-download.test.tsx',
    },
    {
      slug: '03-rendering-engines',
      title: 'Markdown / mermaid / KaTeX / d3-force graph rendering across engines',
      scope: 'mermaid SVG rendering + async init, KaTeX font/CSS loading, react-markdown output, the d3-force graph page (canvas, requestAnimationFrame, pointer events) — WebKit/Gecko divergence, large-document performance on mobile.',
      sources: 'frontend/src/components/note-page/renderers.tsx, frontend/src/components/note-page/RendererComponents.tsx, frontend/src/components/GraphPage.tsx, frontend/src/markdown.ts',
    },
    {
      slug: '04-matchmedia-theme-touch',
      title: 'matchMedia / theme / touch vs hover vs pointer',
      scope: 'useIsMobile matchMedia, useTheme prefers-color-scheme + override, theme-color meta vs dark mode, :hover traps on touch, pointer vs touch vs mouse, gesture conflicts.',
      sources: 'frontend/src/app/useIsMobile.ts, frontend/src/app/useTheme.ts, frontend/src/app/AppTopbar.tsx, frontend/src/styles/topbar.css',
    },
    {
      slug: '05-responsive-safe-area',
      title: 'Responsive layout / safe-area / viewport / RTL / font-scaling',
      scope: 'viewport meta, 100vh vs 100dvh, safe-area-inset in standalone PWA, keyboard-overlap on focus, RTL, user font-scaling/zoom, small-viewport overflow.',
      sources: 'frontend/src/styles/responsive.css, frontend/src/styles/base.css, frontend/src/styles/layout-explorer.css, frontend/src/styles/topbar.css, frontend/index.html',
    },
    {
      slug: '06-browser-api-compat',
      title: 'Browser-API compatibility',
      scope: 'native <dialog>, date/time inputs, crypto/randomUUID, structuredClone, Web Share, IntersectionObserver/ResizeObserver, AbortController, unguarded new APIs — anything with uneven Safari/Firefox/older-WebView support.',
      sources: 'frontend/src/components/ui.tsx, frontend/src/components/NoteActionsDialog.tsx, frontend/src/components/SearchDialog.tsx, frontend/src/components/note-page/frontmatter.ts',
    },
    {
      slug: '07-network-auth-seam',
      title: 'Network / fetch / auth-token / CORS / error-shape contract with the Rust API',
      scope: 'fetch timeout/abort/retry, offline + flaky-network handling, CORS, auth token flow (TokenPrompt + storage), and the error-shape seam with the Rust backend (status codes / JSON ErrorResponse the server actually returns).',
      sources: 'frontend/src/api.ts, frontend/src/writeApi.ts, frontend/src/components/TokenPrompt.tsx, src/app_state.rs',
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
//  (6) categories run concurrently (parallel); per-category disk checkpoints
//      keep resume interrupt-safe regardless of ordering.
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

Return {"findings": [...]} via the schema. Do NOT write any files — the engine persists your findings.`
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
    findings = (r.findings || []).map((f, i) => ({ ...f, id: i })) // (2) stamp stable ids
    await writeBlob(F, JSON.stringify(findings, null, 2), `ckpt-find:${cat.slug}`)
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

// (6) categories run concurrently; each self-checkpoints so resume stays interrupt-safe.
const settled = await parallel(CONFIG.categories.map((cat) => () => runCategory(cat)))
const results = settled.filter(Boolean)
const failed = CONFIG.categories.length - results.length
if (failed > 0) log(`⚠ ${failed} category(ies) did not complete this run — rerun to resume them (finder work is cached).`)

phase('Rollup')
if (results.length > 0) {
  await writeBlob(`${DIR}/SUMMARY.md`, buildSummary(results), 'rollup:SUMMARY')
  log(`SUMMARY.md written from ${results.length} categories`)
}

return { done: results.map((r) => r.cat.slug), failed }
