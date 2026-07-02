export const meta = {
  name: 'client-edge-case-audit',
  description: 'Resumable, adversarially-verified audit of client/browser edge cases in the Hatchdoor React PWA frontend (batched, severity-gated, checkpointed)',
  phases: [
    { title: 'Scan', detail: 'detect which categories are already done on disk' },
    { title: 'Find', detail: 'one Opus-medium finder per remaining category (checkpointed to state/)', model: 'opus' },
    { title: 'Verify', detail: 'diverse-lens panel (code-truth / device-repro / already-handled) per category, severity-gated, Sonnet-medium', model: 'sonnet' },
    { title: 'Write', detail: 'Haiku scribe writes the final category report' },
    { title: 'Summarize', detail: 'regenerate SUMMARY.md' },
  ],
}

const DIR = '/home/battermanz/coding/hatchdoor/docs/audits/client-edge-cases'
const STATE = `${DIR}/state`
const TARGETS = 'Desktop Chrome, Edge, Firefox, desktop Safari (WebKit), iOS Safari + installed PWA (WebKit), Android Chrome'

const CATEGORIES = [
  {
    slug: '01-service-worker-pwa',
    title: 'Service worker / PWA / offline / install / cache staleness',
    scope: 'Workbox service worker (vite-plugin-pwa, registerType autoUpdate), the registerSW onNeedRefresh that calls window.location.reload(), navigateFallback + denylist behaviour, offline behaviour, cache staleness across deploys, install/standalone lifecycle, update races that could reload mid-edit and lose unsaved draft state.',
    sources: 'frontend/vite.config.ts, frontend/src/main.tsx, frontend/src/writeDrafts.ts (unsaved drafts vs reload), frontend/src/app/storage.ts',
  },
  {
    slug: '02-clipboard-upload-download',
    title: 'Clipboard / image upload / file download',
    scope: 'navigator.clipboard availability + permissions across engines, copy fallbacks, image paste/upload, the file-download path (download attribute is ignored on iOS standalone PWA — known prior bug), blob URL lifetime, MIME handling.',
    sources: 'frontend/src/clipboard.ts, frontend/src/imageUpload.ts, frontend/src/App.links-download.test.tsx and the download code it exercises',
  },
  {
    slug: '03-rendering-engines',
    title: 'Markdown / mermaid / KaTeX / d3-force graph rendering across engines',
    scope: 'mermaid SVG rendering and async init, KaTeX font/CSS loading, react-markdown + remark-gfm/math + rehype-katex output, the d3-force graph page (canvas/SVG, requestAnimationFrame, pointer events) — focus on WebKit (Safari/iOS) and Gecko (Firefox) divergence: SVG foreignObject, font metrics, dynamic import timing, large-document performance on mobile.',
    sources: 'frontend/src/components/note-page/renderers.tsx, frontend/src/components/note-page/RendererComponents.tsx, frontend/src/components/GraphPage.tsx, frontend/src/markdown.ts, frontend/src/components/note-page/NotePreview.tsx',
  },
  {
    slug: '04-matchmedia-theme-touch',
    title: 'matchMedia / theme / touch vs hover vs pointer',
    scope: 'useIsMobile matchMedia listener correctness, useTheme prefers-color-scheme + localStorage + manual override, theme-color meta vs dark mode, :hover traps on touch devices, pointer vs touch vs mouse event handling, double-tap zoom, gesture conflicts in the explorer/editor.',
    sources: 'frontend/src/app/useIsMobile.ts, frontend/src/app/useTheme.ts, frontend/src/app/AppTopbar.tsx, frontend/src/app/ExplorerPane.tsx, frontend/src/styles/topbar.css',
  },
  {
    slug: '05-responsive-safe-area',
    title: 'Responsive layout / safe-area / viewport / RTL / font-scaling',
    scope: 'viewport meta, 100vh vs 100dvh on mobile Safari, safe-area-inset (notch/home indicator) in standalone PWA, keyboard-overlap on focus, RTL, user font-size scaling / zoom, small-viewport overflow, scroll locking.',
    sources: 'frontend/src/styles/responsive.css, frontend/src/styles/base.css, frontend/src/styles/layout-explorer.css, frontend/src/styles/topbar.css, frontend/index.html',
  },
  {
    slug: '06-browser-api-compat',
    title: 'Browser-API compatibility',
    scope: 'native <dialog> support/polyfill needs, date/time inputs, crypto/randomUUID, structuredClone, Web Share, IntersectionObserver/ResizeObserver, AbortController, optional chaining of new APIs without guards — anything used that has uneven support on Safari/Firefox or older mobile WebViews.',
    sources: 'frontend/src/components/ui.tsx, frontend/src/components/NoteActionsDialog.tsx, frontend/src/components/SearchDialog.tsx, frontend/src/components/note-page/frontmatter.ts, frontend/src/components/note-page/autocomplete.ts',
  },
  {
    slug: '07-network-auth-seam',
    title: 'Network / fetch / auth-token / CORS / error-shape contract with the Rust API',
    scope: 'fetch timeout/abort/retry, offline + flaky-network handling, CORS, the auth token flow (TokenPrompt + storage), and the contract seam with the Rust backend: does the frontend assume status codes / JSON error shapes (ErrorResponse) that the server actually returns? Mismatches in error handling, 401/403 handling, request coalescing under slow networks.',
    sources: 'frontend/src/api.ts, frontend/src/writeApi.ts, frontend/src/components/TokenPrompt.tsx, frontend/src/app/storage.ts, and the Rust side: src/app_state.rs (ErrorResponse / run_blocking / refresh) and the axum route handlers',
  },
]

const FINDING_PROPS = {
  title: { type: 'string' },
  severity: { type: 'string', enum: ['critical', 'high', 'medium', 'low'] },
  affectedClients: { type: 'array', items: { type: 'string' } },
  file: { type: 'string' },
  line: { type: 'string' },
  description: { type: 'string' },
  why: { type: 'string' },
  fixSketch: { type: 'string' },
}

const FINDINGS_SCHEMA = {
  type: 'object',
  required: ['findings'],
  properties: {
    findings: { type: 'array', items: { type: 'object', required: Object.keys(FINDING_PROPS), properties: FINDING_PROPS } },
  },
}

// A panelist reviews many findings at once and returns one verdict per finding.
const PANEL_SCHEMA = {
  type: 'object',
  required: ['verdicts'],
  properties: {
    verdicts: {
      type: 'array',
      items: {
        type: 'object',
        required: ['title', 'refuted', 'reason'],
        properties: { title: { type: 'string' }, refuted: { type: 'boolean' }, reason: { type: 'string' } },
      },
    },
  },
}

const STATE_SCHEMA = {
  type: 'object',
  required: ['findings', 'panel', 'verified'],
  properties: {
    findings: { type: 'array', items: { type: 'object' } },
    panel: { type: 'array', items: { type: 'object' } }, // [{idx, verdicts:[...]}] — per-panelist checkpoints
    verified: { type: 'array', items: { type: 'object' } },
  },
}

const OK_SCHEMA = { type: 'object', required: ['ok'], properties: { ok: { type: 'boolean' } } }

function finderPrompt(cat) {
  return `You are auditing the Hatchdoor React 19 + TypeScript + Vite PWA frontend for CLIENT/BROWSER edge cases before a public launch.

NOTE: the app has BOTH a read-only vault viewer AND a write mode (create/edit/delete notes, image upload, drafts). Do not assume it is read-only.

Target clients: ${TARGETS}

Category: ${cat.title}
Scope: ${cat.scope}
Primary sources to read (use codegraph_explore and Read on these — repo root is /home/battermanz/coding/hatchdoor): ${cat.sources}

Find REAL edge cases that exist in THIS code — not generic browser lore. Every finding MUST cite an actual file and line you have read. For each, name which specific client(s) misbehave and why (engine-specific reason). Prefer fewer, concrete, code-grounded findings over a long speculative list. If the code is genuinely solid for a sub-area, do not invent issues. Severity reflects launch impact (critical = broken/data-loss on a target client).

CHECKPOINT: After analysis, use the Write tool to save the findings as {"findings": [...]} to ${STATE}/${cat.slug}.findings.json. Then return the same object via the schema.`
}

// Each panelist reviews through ONE independent lens, so the three votes catch
// different failure modes instead of redundantly agreeing (diverse-lens panel).
const LENSES = {
  1: {
    name: 'code-truth',
    focus: `LENS — CODE TRUTH. Read the exact cited lines and decide whether the code, as written, actually does what the finding claims. Refute (refuted=true) if: the cited line/symbol is wrong or stale, the claim misreads the control flow, the mechanism simply isn't in the code, or an in-function guard/branch already prevents it. You are judging "is the mechanism real in this source?" — nothing else.`,
  },
  2: {
    name: 'device-repro',
    focus: `LENS — DEVICE REPRO. Assume the code is as described; decide whether it actually reproduces on the NAMED target client/engine. Refute (refuted=true) if: the named engine (WebKit/Gecko/Blink version, iOS Safari/PWA, Android Chrome, touch vs pointer) does not behave as claimed, the feature is actually supported on all named targets, the affected-client list is wrong, or the trigger path can't occur for a real user on those clients. You are judging "does this bite a real user on the stated device?" — not whether the code is correct.`,
  },
  3: {
    name: 'already-handled',
    focus: `LENS — ALREADY HANDLED / SEVERITY. Decide whether the case is already mitigated or overstated. Refute (refuted=true) if: a fallback, polyfill, error handler, CSS guard, or the upstream library already covers it, another code path makes it unreachable, or the impact is purely cosmetic/trivially recoverable so the severity is wrong. You MAY glance at the cited files' imports to confirm an existing guard, but do NOT explore the wider repo.`,
  },
}

// Batched panelist: apply ONE lens to a list of findings in a single read pass.
function panelPrompt(cat, subset, idx) {
  const lens = LENSES[idx] || LENSES[1]
  const list = subset
    .map((f, n) => `${n + 1}. [${f.severity}] "${f.title}"\n   at ${f.file}:${f.line}\n   claim: ${f.description}\n   reason given: ${f.why}`)
    .join('\n\n')
  const cited = [...new Set(subset.map((f) => f.file))].join(', ')
  return `You are reviewer #${idx} (lens: ${lens.name}) on a panel verifying client edge-case findings for the Hatchdoor frontend (category: ${cat.title}). Repo root: /home/battermanz/coding/hatchdoor.

EFFICIENCY: read ONLY the specific cited files/line ranges below — do NOT explore the wider codebase. The cited files are: ${cited}. Read each one once; it backs several findings.

NOTE: the app has both a read-only viewer AND a write mode (create/edit/delete/upload). Do not refute solely because you assume it is read-only.

${lens.focus}

Judge each finding THROUGH YOUR LENS ONLY. Set refuted=true only when your lens exposes a genuine reason the finding is not a real defect on a named target client. Set refuted=false if, from your lens, it holds up — or if your lens is simply not the right angle to judge it (another reviewer covers the other angles). Do not refute for reasons outside your lens.

Findings to review:
${list}

Return one verdict per finding (match by exact title): {title, refuted, reason}. State your lens's specific reason.`
}

phase('Scan')
const scan = await agent(
  `List the files in ${DIR} (use: ls -la ${DIR}). Return the set of category slugs whose "<slug>.md" FINAL report file already exists and is non-empty. Candidate slugs: ${CATEGORIES.map((c) => c.slug).join(', ')}.`,
  { label: 'scan-ledger', phase: 'Scan', model: 'haiku', effort: 'low', schema: { type: 'object', required: ['done'], properties: { done: { type: 'array', items: { type: 'string' } } } } },
)

const doneSet = new Set((scan && scan.done) || [])
const remaining = CATEGORIES.filter((c) => !doneSet.has(c.slug))
log(`${doneSet.size} categories complete; running ${remaining.length}: ${remaining.map((c) => c.slug).join(', ') || '(none)'}`)

for (const cat of remaining) {
  // --- Load checkpointed state (cheap Haiku reader) ---
  const st = await agent(
    `Read these three files if they exist (use: cat <file> 2>/dev/null) and parse each as JSON.
- ${STATE}/${cat.slug}.findings.json -> {"findings":[...]} (or a bare array)
- ${STATE}/${cat.slug}.panel.json -> an array of {"idx":N,"verdicts":[...]} per-panelist checkpoints
- ${STATE}/${cat.slug}.verdicts.json -> an array of already-verified finding objects
Return {"findings": <findings array or []>, "panel": <panel array or []>, "verified": <verified array or []>}. Missing/empty file -> []. Do not invent data.`,
    { label: `load:${cat.slug}`, phase: 'Find', model: 'haiku', effort: 'low', schema: STATE_SCHEMA },
  )

  let findings = (st && st.findings) || []
  let panel = (st && st.panel) || [] // [{idx, verdicts}]
  let verified = (st && st.verified) || []

  // --- Stage 1: find (Opus-medium), only if not checkpointed ---
  if (findings.length === 0) {
    log(`▶ ${cat.slug}: finding (Opus-medium)…`)
    const r = await agent(finderPrompt(cat), { label: `find:${cat.slug}`, phase: 'Find', model: 'opus', effort: 'medium', schema: FINDINGS_SCHEMA })
    findings = (r && r.findings) || []
  } else {
    log(`▶ ${cat.slug}: resumed — ${findings.length} findings, ${panel.length} panelist(s) done, verified=${verified.length}`)
  }

  // --- Stage 2: batched, severity-gated adversarial panel (only if not done) ---
  if (verified.length === 0 && findings.length > 0) {
    const sev = (f) => f.severity
    const highCrit = findings.filter((f) => sev(f) === 'critical' || sev(f) === 'high')
    const medium = findings.filter((f) => sev(f) === 'medium')

    // Panelist subsets: p1 (code-truth lens) covers high/crit + medium (medium gets that 1 lens);
    // p2 (device-repro) and p3 (already-handled) cover high/crit only => 3 diverse lenses on high/crit.
    const panelists = []
    if (highCrit.length > 0) {
      panelists.push({ idx: 1, subset: [...highCrit, ...medium] })
      panelists.push({ idx: 2, subset: highCrit })
      panelists.push({ idx: 3, subset: highCrit })
    } else if (medium.length > 0) {
      panelists.push({ idx: 1, subset: medium })
    }
    log(`  ${cat.slug}: ${highCrit.length} high/crit (3 diverse lenses), ${medium.length} medium (code-truth lens), ${findings.length - highCrit.length - medium.length} low (unverified); ${panelists.length} panelist(s)`)

    // SEQUENTIAL: run each panelist one at a time, checkpoint panel.json after each,
    // so an interrupt loses at most a single panelist.
    const doneIdx = new Set(panel.map((p) => p.idx))
    for (const p of panelists) {
      if (doneIdx.has(p.idx)) continue
      const r = await agent(panelPrompt(cat, p.subset, p.idx), { label: `panel:${cat.slug}#${p.idx}`, phase: 'Verify', model: 'sonnet', effort: 'medium', schema: PANEL_SCHEMA })
      panel.push({ idx: p.idx, verdicts: (r && r.verdicts) || [] })

      const panelBlob = JSON.stringify(panel, null, 2)
      await agent(
        `Use the Write tool to write the following EXACT text to ${STATE}/${cat.slug}.panel.json. Copy byte-for-byte: do not change, summarize, reformat, or comment. Then return {"ok": true}.

<<<JSON
${panelBlob}
JSON`,
        { label: `ckpt-panel:${cat.slug}#${p.idx}`, phase: 'Verify', model: 'haiku', effort: 'low', schema: OK_SCHEMA },
      )
      log(`    ${cat.slug}: panelist ${p.idx}/${panelists.length} done & checkpointed`)
    }

    // Aggregate verdicts across panelists by finding title.
    const verdictsFor = (title) => panel.map((pr) => (pr.verdicts || []).find((v) => v.title === title)).filter(Boolean)
    verified = findings.map((f) => {
      const s = sev(f)
      if (s === 'low') return { ...f, survives: true, status: 'unverified', refutes: 0, votes: 0, verdicts: [] }
      const vs = verdictsFor(f.title)
      const refutes = vs.filter((v) => v.refuted).length
      const needed = s === 'medium' ? 1 : 2 // medium: 1 refute kills; high/crit: majority of 3
      return { ...f, survives: refutes < needed, status: 'verified', refutes, votes: vs.length, verdicts: vs }
    })

    // checkpoint final aggregated verdicts (verify-complete marker)
    const blob = JSON.stringify(verified, null, 2)
    await agent(
      `Use the Write tool to write the following EXACT text to ${STATE}/${cat.slug}.verdicts.json. Copy byte-for-byte: do not change, summarize, reformat, or comment. Then return {"ok": true}.

<<<JSON
${blob}
JSON`,
      { label: `ckpt:${cat.slug}`, phase: 'Verify', model: 'haiku', effort: 'low', schema: OK_SCHEMA },
    )
  }

  // --- Stage 3: final report (Haiku) ---
  const confirmed = verified.filter((v) => v.survives)
  const rejected = verified.filter((v) => !v.survives)
  const payload = JSON.stringify({ confirmed, rejected }, null, 2)
  await agent(
    `Write a markdown audit report to ${DIR}/${cat.slug}.md using the Write tool. Category title: "${cat.title}".

Structure:
- H1 with the category title.
- One-line summary: N confirmed (by severity), M refuted. Note that LOW findings are included as "unverified" (panel did not vote on them).
- H2 "Confirmed findings": for EACH confirmed finding an H3 "SEVERITY: title" (append " (unverified)" if its status is unverified), then bullets: Affected clients, Location (\`file:line\`), What happens, Why (engine reason), Fix sketch. Order critical > high > medium > low.
- H2 "Refuted (not real / already handled)": each rejected finding title + one-line reason from its verdicts.

Data (JSON, each finding has severity/status/verdicts):
${payload}

After writing, return {"ok": true}.`,
    { label: `write:${cat.slug}`, phase: 'Write', model: 'haiku', effort: 'low', schema: OK_SCHEMA },
  )
  log(`✔ ${cat.slug}: banked (${confirmed.length} confirmed, ${rejected.length} refuted)`)
}

phase('Summarize')
await agent(
  `Read every NN-*.md category report in ${DIR} (files named NN-*.md, NOT _manifest.md, NOT SUMMARY.md, NOT anything under state/). Then write ${DIR}/SUMMARY.md with the Write tool: a launch-readiness rollup with a table of all confirmed findings across categories sorted by severity (columns: Severity, Category, Title, Affected clients, Location), plus a "Top launch blockers" section listing the critical/high items. Mark unverified (low) rows as such. Keep it concise.`,
  { label: 'summary', phase: 'Summarize', model: 'haiku', effort: 'low' },
)

return { done: [...doneSet], ran: remaining.map((c) => c.slug) }
