import { useEffect, useState } from "react";
import { Link } from "react-router-dom";

import { apiFetch } from "../api/api";
import { readErrorMessage } from "../api/apiError";
import { useVaultScope } from "../hooks/useVaultScope";
import { useVaultCollection } from "../vaults";

import { StateBlock } from "./ui";
import type {
  CreatedDateStatus,
  FolderStat,
  LinkedNoteRef,
  MonthActivity,
  NoteRef,
  NoteWordRef,
  TagStat,
  VaultId,
  VaultQualifiedStats,
  VaultScope,
  VaultStats,
  VaultSummary,
} from "../types";

function fmtNum(n: number): string {
  return n.toLocaleString();
}

function fmtBytes(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)} MB`;
  return `${(n / 1_000).toFixed(0)} KB`;
}

function fmtMonth(m: string): string {
  const [year, month] = m.split("-");
  const date = new Date(Number(year), Number(month) - 1, 1);
  return date.toLocaleString("en", { month: "short" });
}

/**
 * The largest value in `arr`, floored at 1 so it is always safe to divide a
 * bar's value by. A window nobody wrote in has a maximum of zero, and scaling
 * against that asks for a NaN height.
 */
function maxOf(arr: number[]): number {
  return Math.max(1, ...arr);
}

/**
 * How many calendar months the "Notes created" window spans, matching the
 * backend's window. The chart draws whatever the backend sends, but it
 * averages over this and names it in the section heading.
 */
const ACTIVITY_WINDOW_MONTHS = 6;

/**
 * How long to wait before asking again for a Vault whose history the server
 * was still reading (#300). The server itself waits a moment for the walk
 * before answering, so this only paces a long one.
 */
const HISTORY_RETRY_MS = 3000;

/**
 * How many times to ask again before leaving the notice up. A walk that has
 * not finished in about a minute and a half is not worth polling for; the next
 * visit to the page asks afresh.
 */
const HISTORY_RETRY_LIMIT = 20;

/** The line under the chart when some created dates are not what they should be. */
const CREATED_DATE_NOTICE: Record<CreatedDateStatus, string | null> = {
  complete: null,
  estimated:
    "Some dates are estimated: this repository's history is incomplete.",
  reading: "Reading history…",
};

function SectionHead({ num, title }: { num: string; title: string }) {
  return (
    <div className="stats-section-head">
      <span className="stats-section-num">{num}</span>
      <span className="stats-section-title">{title}</span>
    </div>
  );
}

function TagBars({ tags }: { tags: TagStat[] }) {
  const top = tags.slice(0, 20);
  const max = maxOf(top.map((t) => t.note_count));
  return (
    <div className="stats-bar-list">
      {top.map((t) => (
        <div key={t.tag} className="stats-bar-row">
          <span className="stats-bar-label">{t.tag}</span>
          <div className="stats-bar-track">
            <div
              className="stats-bar-fill"
              style={{ width: `${(t.note_count / max) * 100}%` }}
            />
          </div>
          <span className="stats-bar-count">{t.note_count}</span>
        </div>
      ))}
    </div>
  );
}

function RankedList({
  vaultId,
  notes,
}: {
  vaultId: VaultId;
  notes: LinkedNoteRef[];
}) {
  return (
    <div className="stats-ranked-list">
      {notes.map((n, i) => (
        <div key={n.slug} className="stats-ranked-row">
          <span className="stats-ranked-rank">{i + 1}</span>
          <Link
            className="stats-ranked-title"
            to={`/v/${encodeURIComponent(vaultId)}/n/${n.slug}`}
          >
            {n.title}
          </Link>
          <span className="stats-ranked-meta">
            {n.backlink_count} backlinks
          </span>
        </div>
      ))}
    </div>
  );
}

function ActivityChart({
  months,
  status,
}: {
  months: MonthActivity[];
  status: CreatedDateStatus;
}) {
  if (months.length === 0) return null;
  const max = maxOf(months.map((m) => m.created_count));
  const peak = months.reduce((a, b) =>
    a.created_count >= b.created_count ? a : b,
  );
  // Over the window, not over the bars that arrived. The backend guarantees
  // one entry per month of the window, so the two are equal today; dividing by
  // the window is what keeps the figure honest if they ever part.
  const avg =
    months.reduce((s, m) => s + m.created_count, 0) / ACTIVITY_WINDOW_MONTHS;
  const notice = CREATED_DATE_NOTICE[status];

  return (
    <>
      <div className="stats-activity">
        {months.map((m) => (
          <div key={m.month} className="stats-act-col">
            <span className="stats-act-count">{m.created_count}</span>
            <div
              className="stats-act-bar"
              style={{
                height: `${Math.max(2, (m.created_count / max) * 68)}px`,
              }}
            />
            <span className="stats-act-month">{fmtMonth(m.month)}</span>
          </div>
        ))}
      </div>
      <div className="stats-activity-meta">
        <span>
          Peak: {fmtMonth(peak.month)} · {peak.created_count} new{" "}
          {peak.created_count === 1 ? "note" : "notes"}
        </span>
        <span>Avg: {avg.toFixed(1)} / month</span>
      </div>
      {notice ? (
        <div className="stats-activity-meta" role="status">
          <span>{notice}</span>
        </div>
      ) : null}
    </>
  );
}

function FolderList({ folders }: { folders: FolderStat[] }) {
  return (
    <div className="stats-folder-list">
      {folders.map((f) => (
        <div key={f.folder} className="stats-folder-row">
          <span className="stats-folder-name">
            <span className="stats-folder-slash">
              {f.folder === "(root)" ? "·" : "/"}
            </span>{" "}
            {f.folder === "(root)" ? "root" : f.folder}
          </span>
          <span className="stats-folder-count">{f.note_count}</span>
        </div>
      ))}
    </div>
  );
}

function NoteWordList({
  vaultId,
  notes,
  sublabel,
}: {
  vaultId: VaultId;
  notes: NoteWordRef[];
  sublabel: string;
}) {
  return (
    <>
      <p className={`stats-sublabel${sublabel === "Longest" ? " hot" : ""}`}>
        {sublabel}
      </p>
      <div className="stats-note-list" style={{ marginBottom: "0.9rem" }}>
        {notes.map((n) => (
          <div key={n.slug} className="stats-note-row">
            <Link
              className="stats-note-title"
              to={`/v/${encodeURIComponent(vaultId)}/n/${n.slug}`}
            >
              {n.title}
            </Link>
            <span className="stats-note-meta">{fmtNum(n.word_count)} w</span>
          </div>
        ))}
      </div>
    </>
  );
}

function PillList({ vaultId, notes }: { vaultId: VaultId; notes: NoteRef[] }) {
  if (notes.length === 0) {
    return (
      <p
        style={{
          fontFamily: "var(--font-mono)",
          fontSize: "0.72rem",
          color: "var(--muted)",
        }}
      >
        None found.
      </p>
    );
  }
  return (
    <div className="stats-pill-list">
      {notes.map((n) => (
        <Link
          key={n.slug}
          className="stats-pill"
          to={`/v/${encodeURIComponent(vaultId)}/n/${n.slug}`}
        >
          {n.title}
        </Link>
      ))}
    </div>
  );
}

function RecentList({
  vaultId,
  notes,
}: {
  vaultId: VaultId;
  notes: NoteRef[];
}) {
  return (
    <div className="stats-note-list">
      {notes.map((n) => (
        <div key={n.slug} className="stats-note-row">
          <Link
            className="stats-note-title"
            to={`/v/${encodeURIComponent(vaultId)}/n/${n.slug}`}
          >
            {n.title}
          </Link>
        </div>
      ))}
    </div>
  );
}

/** One Vault's answer: its numbers, or the reason it could not give them. */
interface VaultStatsResult {
  vault: VaultSummary;
  stats: VaultStats | null;
  error: string | null;
}

/** One Vault's `stats/detail` read, with a failure kept as that Vault's answer. */
async function loadVaultStats(vault: VaultSummary): Promise<VaultStatsResult> {
  try {
    const res = await apiFetch(
      `/api/v1/vaults/${encodeURIComponent(vault.vault_id)}/stats/detail`,
    );
    if (!res.ok) {
      throw new Error(await readErrorMessage(res, "Stats failed"));
    }
    const projection = (await res.json()) as VaultQualifiedStats;
    return { vault, stats: projection.stats, error: null };
  } catch (err) {
    return {
      vault,
      stats: null,
      error:
        err instanceof Error
          ? err.message
          : "Failed to load stats for this Vault.",
    };
  }
}

/**
 * Every Vault the current scope covers, in Vault-management order: the one
 * named Vault when scope is narrowed, every enabled Vault under `"all"`.
 * A narrowed scope naming a Vault that discovery no longer returns (it was
 * disabled or removed in another tab) covers nothing, which the caller states
 * rather than silently falling back to a Vault the reader did not ask for.
 */
function vaultsInScope(
  scope: VaultScope,
  vaults: VaultSummary[],
): VaultSummary[] {
  if (scope === "all") {
    return vaults;
  }
  const narrowed = vaults.find((vault) => vault.vault_id === scope);
  return narrowed ? [narrowed] : [];
}

export function StatsPage() {
  const [scope] = useVaultScope();
  const {
    vaults,
    loading: loadingVaults,
    readState,
    error: discoveryError,
    refresh: reloadVaults,
  } = useVaultCollection();
  const targets = vaultsInScope(scope, vaults);
  // Statistics stay grouped per Vault (#62) and `stats/detail` is an exact
  // single-Vault read, so `all` is N reads presented as N sections rather
  // than one merged total that would silently add unlike Vaults together.
  const targetIds = targets.map((vault) => vault.vault_id).join(",");
  const [results, setResults] = useState<VaultStatsResult[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (loadingVaults) {
      return;
    }
    if (targets.length === 0) {
      setResults([]);
      setError(null);
      setLoading(false);
      return;
    }

    let cancelled = false;
    void (async () => {
      setLoading(true);
      setError(null);
      try {
        const loaded = await Promise.all(targets.map(loadVaultStats));
        if (!cancelled) setResults(loaded);
      } catch (err) {
        if (!cancelled)
          setError(err instanceof Error ? err.message : "Failed to load stats");
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
    // `targetIds` stands in for `targets`, which is a fresh array each render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loadingVaults, targetIds]);

  // A Vault whose history the server was still reading is asked again, alone
  // and in place, until its chart is final or the retries run out (#300). A
  // retry that fails keeps the answer already on screen rather than replacing
  // it with an error.
  const [historyRetries, setHistoryRetries] = useState(0);
  useEffect(() => {
    const reading = results.filter(
      (result) => result.stats?.created_date_status === "reading",
    );
    if (reading.length === 0 || historyRetries >= HISTORY_RETRY_LIMIT) {
      return;
    }
    let cancelled = false;
    const timer = window.setTimeout(() => {
      void (async () => {
        const refreshed = await Promise.all(
          reading.map((result) => loadVaultStats(result.vault)),
        );
        if (cancelled) return;
        setHistoryRetries((count) => count + 1);
        setResults((current) =>
          current.map((result) => {
            const next = refreshed.find(
              (candidate) =>
                candidate.vault.vault_id === result.vault.vault_id &&
                candidate.stats !== null,
            );
            return next ?? result;
          }),
        );
      })();
    }, HISTORY_RETRY_MS);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [results, historyRetries]);

  if (loadingVaults || loading) {
    return (
      <div className="stats-loading">
        <div className="stats-loading-strip" />
        <div className="stats-loading-block" />
        <div className="stats-loading-block" />
      </div>
    );
  }

  // No Vault list to scope to because discovery failed, not because there are
  // none (#333).
  if (readState === "error") {
    return (
      <StateBlock
        tone="error"
        title="Stats Unavailable"
        description={discoveryError ?? "Could not load your Vaults."}
        actionLabel="Try again"
        onAction={() => void reloadVaults()}
      />
    );
  }

  if (targets.length === 0) {
    return (
      <StateBlock
        title="Stats Unavailable"
        description={
          scope === "all"
            ? "No Vault is available to show statistics for."
            : "The Vault this page is scoped to is no longer available."
        }
      />
    );
  }

  if (error) {
    return <StateBlock title="Stats Unavailable" description={error} />;
  }

  // A single-Vault answer keeps the page it has always had; only a scope that
  // genuinely spans more than one Vault earns the per-Vault section heading.
  const grouped = results.length > 1;

  return (
    <div className="stats-page">
      {/* Header */}
      <div className="stats-page-header">
        <p className="stats-eyebrow">
          Vault · {new Date().toISOString().slice(0, 10)}
        </p>
        <h1 className="stats-title">Stats</h1>
      </div>

      {results.map((result) => (
        <section
          key={result.vault.vault_id}
          className={grouped ? "stats-vault-section" : undefined}
        >
          {grouped ? (
            <h2 className="stats-vault-heading">{result.vault.name}</h2>
          ) : null}
          {result.stats ? (
            <VaultStatsReport
              vaultId={result.vault.vault_id}
              stats={result.stats}
            />
          ) : (
            <StateBlock
              title="Stats Unavailable"
              description={result.error ?? "Could not load vault stats."}
            />
          )}
        </section>
      ))}
    </div>
  );
}

function VaultStatsReport({
  vaultId,
  stats,
}: {
  vaultId: VaultId;
  stats: VaultStats;
}) {
  return (
    <>
      {/* Metric strip */}
      <div className="stats-metric-strip">
        <div className="stats-metric-cell">
          <div className="stats-metric-num">{fmtNum(stats.note_count)}</div>
          <div className="stats-metric-label">Notes</div>
        </div>
        <div className="stats-metric-cell">
          <div className="stats-metric-num">{fmtNum(stats.word_count)}</div>
          <div className="stats-metric-label">Words</div>
        </div>
        <div className="stats-metric-cell">
          <div className="stats-metric-num">{fmtNum(stats.tag_count)}</div>
          <div className="stats-metric-label">Tags</div>
        </div>
        <div className="stats-metric-cell">
          <div className="stats-metric-num">{fmtNum(stats.link_count)}</div>
          <div className="stats-metric-label">Links</div>
        </div>
        <div className="stats-metric-cell">
          <div className="stats-metric-num">{fmtNum(stats.image_count)}</div>
          <div className="stats-metric-label">Images</div>
        </div>
      </div>

      {/* Row 1: Tags + Most Linked */}
      <div className="stats-two-col" style={{ marginBottom: "2rem" }}>
        <div className="stats-section">
          <SectionHead num="01" title="Top Tags" />
          <TagBars tags={stats.top_tags} />
        </div>
        <div className="stats-section">
          <SectionHead num="02" title="Most Linked Notes" />
          <RankedList vaultId={vaultId} notes={stats.most_linked} />
        </div>
      </div>

      {/* Row 2: Notes created, by created date rather than last edit (#300) */}
      <div className="stats-section" style={{ marginBottom: "2rem" }}>
        <SectionHead
          num="03"
          title={`Notes created, last ${ACTIVITY_WINDOW_MONTHS} months`}
        />
        <ActivityChart
          months={stats.activity_by_month}
          status={stats.created_date_status}
        />
      </div>

      {/* Row 3: Folders + Word extremes */}
      <div className="stats-two-col" style={{ marginBottom: "2rem" }}>
        <div className="stats-section">
          <SectionHead num="04" title="Notes per Folder" />
          <FolderList folders={stats.notes_per_folder} />
        </div>
        <div className="stats-section">
          <SectionHead num="05" title="Word Count Extremes" />
          <NoteWordList
            vaultId={vaultId}
            notes={stats.longest_notes}
            sublabel="Longest"
          />
          <NoteWordList
            vaultId={vaultId}
            notes={stats.shortest_notes}
            sublabel="Shortest"
          />
        </div>
      </div>

      {/* Row 4: Link Balance + Averages + Modified this week */}
      <div className="stats-three-col" style={{ marginBottom: "2rem" }}>
        <div className="stats-section">
          <SectionHead num="06" title="Link Balance" />
          <div className="stats-twin">
            <div className="stats-twin-cell">
              <div className="stats-twin-num">
                {fmtNum(stats.total_outgoing_links)}
              </div>
              <div className="stats-twin-lbl">Outgoing</div>
            </div>
            <div className="stats-twin-cell">
              <div className="stats-twin-num">
                {fmtNum(stats.total_backlinks)}
              </div>
              <div className="stats-twin-lbl">Backlinks</div>
            </div>
          </div>
        </div>
        <div className="stats-section">
          <SectionHead num="07" title="Averages" />
          <div className="stats-single">
            <div className="stats-single-num">
              {fmtNum(stats.avg_word_count)}
            </div>
            <div className="stats-single-lbl">Avg words / note</div>
          </div>
          <div className="stats-single">
            <div className="stats-single-num">
              {fmtBytes(stats.vault_size_bytes)}
            </div>
            <div className="stats-single-lbl">Vault size on disk</div>
          </div>
        </div>
        <div className="stats-section">
          <SectionHead num="08" title="Modified This Week" />
          <div className="stats-big-count">
            {stats.modified_this_week.count}
          </div>
          <div className="stats-big-count-lbl">notes</div>
          <RecentList
            vaultId={vaultId}
            notes={stats.modified_this_week.notes.slice(0, 5)}
          />
        </div>
      </div>

      {/* Row 5: Orphans + No Tags */}
      <div className="stats-two-col">
        <div className="stats-section">
          <SectionHead num="09" title="Orphan Notes" />
          <p
            style={{
              fontFamily: "var(--font-mono)",
              fontSize: "0.66rem",
              color: "var(--muted)",
              marginBottom: "0.65rem",
              lineHeight: "1.5",
            }}
          >
            No incoming or outgoing links.
          </p>
          <PillList vaultId={vaultId} notes={stats.orphan_notes} />
        </div>
        <div className="stats-section">
          <SectionHead num="10" title="Notes with No Tags" />
          <p
            style={{
              fontFamily: "var(--font-mono)",
              fontSize: "0.66rem",
              color: "var(--muted)",
              marginBottom: "0.65rem",
              lineHeight: "1.5",
            }}
          >
            Missing all tags.
          </p>
          <PillList vaultId={vaultId} notes={stats.no_tag_notes} />
        </div>
      </div>
    </>
  );
}
