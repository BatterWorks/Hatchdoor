import { apiFetch, withAccessToken } from "../api/api";
import { readErrorMessage } from "../api/apiError";
import type {
  LegacyMigrationRecovery,
  VaultDiscoveryResponse,
  VaultId,
  VaultRegistryRecovery,
  VaultStatistics,
  VaultSummary,
  VaultReadProjection,
} from "../types";

/**
 * Everything the app knows about the Vault collection at one instant.
 *
 * `vaults` is the browsing list — enabled Vaults only, in the order
 * `GET /api/v1/vaults` returns them. Disabled Vaults never appear there and
 * never participate in `"all"` (docs/migrations/vault-scoped-clients.md).
 * `allVaults` is the registry list, disabled Vaults included, which only Vault
 * management has any business rendering.
 *
 * `recovery` (the persisted registry file itself is unreadable) and
 * `legacyMigrationRecovery` (the registry loaded fine, empty, but a failed safe
 * legacy import still needs recovery) are mutually exclusive broken-start
 * conditions (#150): both leave the lists empty, but only one is ever set.
 *
 * `revision` is the collection revision the state reflects: seeded from the
 * discovery response and advanced by the SSE stream. `null` until a discovery
 * lands, which is a different fact from a server sitting at revision 0 — the
 * two shared the `0` sentinel until a freshly restarted server was found to
 * spend its first genuine change being mistaken for "nothing known yet".
 *
 * `readState` is the one answer to "what do we know about the collection",
 * derived from the fields above so no consumer has to reassemble it (#333):
 *
 * - `loading`: no discovery has answered yet.
 * - `error`: discovery failed and none has ever succeeded. The lists are empty
 *   because nothing is known, not because the registry is empty, so this must
 *   never render as the zero-Vault state or be taken as evidence that a stored
 *   Vault has left the collection.
 * - `empty`: discovery succeeded with no enabled Vaults (a broken registry is
 *   also `empty`; `recovery`/`legacyMigrationRecovery` say which).
 * - `partial`: the Vault list is known but the note counts are not all
 *   current: the stats read failed, or answered without some Vault.
 * - `ready`: everything answered.
 *
 * A refresh that fails after a discovery has succeeded keeps the last known
 * list and sets the `error` field, but `readState` stays `empty`, `partial` or
 * `ready` rather than dropping back to `"error"`: the list it shows is still
 * the best answer there is.
 *
 * `noteCounts` holds only counts actually read. A Vault with no entry has an
 * unknown count, which the slot renders as unknown, never as 0.
 */
export type VaultCollectionReadState =
  "loading" | "error" | "empty" | "partial" | "ready";

export type VaultCollectionState = {
  readState: VaultCollectionReadState;
  vaults: VaultSummary[];
  allVaults: VaultSummary[];
  demoMode: boolean;
  loading: boolean;
  error: string | null;
  recovery: VaultRegistryRecovery | null;
  legacyMigrationRecovery: LegacyMigrationRecovery | null;
  registryRevision: number | null;
  revision: number | null;
  noteCounts: Record<VaultId, number>;
  noteCountsPartial: boolean;
};

const EMPTY_STATE: VaultCollectionState = {
  readState: "loading",
  vaults: [],
  allVaults: [],
  demoMode: false,
  loading: true,
  error: null,
  recovery: null,
  legacyMigrationRecovery: null,
  registryRevision: null,
  revision: null,
  noteCounts: {},
  noteCountsPartial: false,
};

type Listener = () => void;

let state: VaultCollectionState = EMPTY_STATE;
const listeners = new Set<Listener>();
let started = false;
let stream: EventSource | null = null;
/** Bumped by every reset so a fetch still in flight over the old collection
 * cannot write its answer into the new one. */
let generation = 0;
/** Whether any discovery has answered for the current generation. Kept apart
 * from `revision`, which the SSE stream can seed before discovery lands. */
let discovered = false;

function deriveReadState(next: VaultCollectionState): VaultCollectionReadState {
  if (!discovered) {
    return next.error ? "error" : "loading";
  }
  if (next.vaults.length === 0) {
    return "empty";
  }
  return next.noteCountsPartial ? "partial" : "ready";
}

/** A refresh that finds nothing new must not hand React a new object: the
 * collection revision bumps on every note write, and a fresh-but-identical
 * Vault list would otherwise relayout the graph and re-run every effect keyed
 * on it. `reuseIfUnchanged` keeps the previous value's identity when the new
 * one says the same thing, and `publish` then drops a patch that changes
 * nothing at all. */
function reuseIfUnchanged<T>(previous: T, next: T): T {
  return JSON.stringify(previous) === JSON.stringify(next) ? previous : next;
}

function publish(patch: Partial<VaultCollectionState>) {
  const merged = { ...state, ...patch };
  const next = { ...merged, readState: deriveReadState(merged) };
  const changed = (Object.keys(next) as (keyof VaultCollectionState)[]).some(
    (key) => !Object.is(state[key], next[key]),
  );
  if (!changed) {
    return;
  }
  state = next;
  for (const listener of [...listeners]) {
    listener();
  }
}

/** The current snapshot. Referentially stable until something changes, so
 * `useSyncExternalStore` can compare it by identity. */
export function getVaultCollectionSnapshot(): VaultCollectionState {
  return state;
}

async function loadNoteCounts(forGeneration: number): Promise<void> {
  try {
    const res = await apiFetch("/api/v1/vaults/all/stats");
    if (forGeneration !== generation) {
      return;
    }
    if (!res.ok) {
      publish({ noteCountsPartial: true });
      return;
    }
    const projection = (await res.json()) as VaultReadProjection<
      VaultStatistics[]
    >;
    if (forGeneration !== generation) {
      return;
    }
    // Merge rather than rebuild: under `all` the server leaves a Vault whose
    // snapshot could not be read out of `data` and lists it only as a
    // participant. Rebuilding from `data` alone dropped its last known count,
    // and the slot then read the gap as "0 notes". A Vault that did not
    // answer keeps what was last known, or stays unknown if nothing was.
    const next: Record<VaultId, number> = { ...state.noteCounts };
    const answered = new Set<VaultId>();
    for (const entry of projection.data ?? []) {
      next[entry.vault_id] = entry.note_count;
      answered.add(entry.vault_id);
    }
    const missing = state.vaults.some((vault) => !answered.has(vault.vault_id));
    publish({
      noteCounts: reuseIfUnchanged(state.noteCounts, next),
      noteCountsPartial: Boolean(projection.partial) || missing,
    });
  } catch {
    if (forGeneration !== generation) {
      return;
    }
    // Leave prior counts in place; the slot treats a missing entry as unknown.
    publish({ noteCountsPartial: true });
  }
}

/** The one `GET /api/v1/vaults` read in the app. Throws the server's own
 * message on a refusal, so each caller decides what to do with it. */
async function readDiscovery(): Promise<VaultDiscoveryResponse> {
  let res: Response;
  try {
    res = await apiFetch("/api/v1/vaults");
  } catch {
    // Offline, a server mid-restart, or a timed-out request: the browser's
    // own wording ("Failed to fetch", "Load failed") names none of them.
    throw new Error("Could not reach the Hatchdoor server.");
  }
  if (!res.ok) {
    throw new Error(await readErrorMessage(res, "Failed loading Vaults"));
  }
  return (await res.json()) as VaultDiscoveryResponse;
}

async function loadCollection(forGeneration: number): Promise<void> {
  try {
    const discovery = await readDiscovery();
    if (forGeneration !== generation) {
      return;
    }
    const allVaults = reuseIfUnchanged(
      state.allVaults,
      Array.isArray(discovery.vaults) ? discovery.vaults : [],
    );
    const enabled = reuseIfUnchanged(
      state.vaults,
      allVaults.filter((vault) => vault.enabled),
    );
    const recovery = discovery.recovery ?? null;
    discovered = true;
    publish({
      allVaults,
      vaults: enabled,
      demoMode: discovery.demo_mode,
      recovery: reuseIfUnchanged(state.recovery, recovery),
      legacyMigrationRecovery: reuseIfUnchanged(
        state.legacyMigrationRecovery,
        discovery.legacy_migration_recovery ?? null,
      ),
      registryRevision: discovery.registry_revision ?? null,
      // Seed the baseline, once, from the read the vaults themselves came
      // from. The stream reports the server's current revision the moment it
      // connects rather than a delta, so against a starting `revision` of 0
      // that first event always read as an invalidation and every consumer
      // keyed on it reloaded: the explorer tree and the recent list were each
      // fetched twice on every page load. Only the baseline is taken here.
      // Once a revision is known the stream alone moves it, which is what
      // keeps a revision counting from zero again after a server restart a
      // change this client follows rather than one a later discovery undoes.
      // The narrow race stays honest either way: a collection that genuinely
      // changed between this response and the stream connecting reports a
      // different revision, and that one still invalidates.
      revision:
        state.revision === null
          ? discovery.collection_revision
          : state.revision,
      error: null,
    });
    // A broken registry has no collection to count, and the stats read would
    // only produce a second error saying the same thing. Neither has an empty
    // browsing list: the `all` scope those counts come from covers enabled
    // Vaults, and every reader of them renders one.
    if (recovery || enabled.length === 0) {
      publish({ noteCountsPartial: false });
      return;
    }
    await loadNoteCounts(forGeneration);
  } catch (err) {
    if (forGeneration !== generation) {
      return;
    }
    publish({
      error:
        err instanceof Error ? err.message : "Unknown Vault discovery error",
    });
  }
}

/**
 * Reload the collection and its counts. Every Vault mutation ends here — the
 * SSE revision bump calls it, and so does a caller that has just written and
 * does not want to wait for the round trip.
 */
export async function refreshVaultCollection(): Promise<void> {
  const forGeneration = generation;
  await loadCollection(forGeneration);
  if (forGeneration === generation) {
    publish({ loading: false });
  }
}

/**
 * The current `expected_registry_revision`, read fresh rather than from the
 * snapshot: a mutation guarded by optimistic concurrency has to compare against
 * what the server holds now, not what the last load happened to see. The
 * snapshot is updated with the answer.
 */
export async function fetchRegistryRevision(): Promise<number | null> {
  try {
    const revision = (await readDiscovery()).registry_revision ?? null;
    publish({ registryRevision: revision });
    return revision;
  } catch {
    return null;
  }
}

function openRevisionStream(): void {
  if (!("EventSource" in window)) {
    return;
  }
  const events = new EventSource(withAccessToken("/api/v1/vaults/events"));
  stream = events;
  events.addEventListener(
    "vault-collection-revision",
    (event: MessageEvent<string>) => {
      let revision: number;
      try {
        const payload = JSON.parse(event.data) as {
          collection_revision?: unknown;
        };
        if (typeof payload.collection_revision !== "number") {
          return;
        }
        revision = payload.collection_revision;
      } catch {
        // Ignore malformed event payloads; the next valid revision resyncs.
        return;
      }
      // `collection_revision` is in-memory and counts from 0 again when the
      // backend restarts, so a revision going backwards means a new server
      // generation, not a stale event. Discarding it would strand the whole app
      // on the old high-water mark: this is the one invalidation path the
      // collection has, so nothing else would ever refresh the list, the
      // counts, or the explorer until the new server counted past it.
      if (revision === state.revision) {
        return;
      }
      publish({ revision });
      void loadCollection(generation);
    },
  );
}

function start(): void {
  started = true;
  openRevisionStream();
  void refreshVaultCollection();
}

/**
 * Watch the collection. The first subscriber starts the single collection load
 * and opens the single SSE subscription the whole app shares; the last one to
 * leave tears both down, so a remounted app loads fresh rather than rendering
 * whatever the previous mount happened to end on.
 */
export function subscribeVaultCollection(listener: Listener): () => void {
  listeners.add(listener);
  if (!started) {
    start();
  }
  return () => {
    listeners.delete(listener);
    if (listeners.size === 0) {
      resetVaultCollection();
    }
  };
}

/**
 * Drop every cached answer and close the stream. The last subscriber leaving is
 * the ordinary caller. Called while subscribers are still watching — a test
 * clearing state between cases, or a caller forcing a cold reload — it tells
 * them the collection is unknown again and starts over, rather than leaving
 * React rendering a snapshot nothing is refreshing any more.
 */
export function resetVaultCollection(): void {
  generation += 1;
  discovered = false;
  started = false;
  stream?.close();
  stream = null;
  state = EMPTY_STATE;
  for (const listener of [...listeners]) {
    listener();
  }
  if (listeners.size > 0) {
    start();
  }
}
