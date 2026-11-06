import { apiFetch, withAccessToken } from "../api/api";
import { readErrorMessage } from "../api/apiError";
import type {
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
 * `recovery` means the persisted registry file itself is unreadable (#150).
 * It leaves the lists empty.
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
 *   also `empty`; `recovery` says which).
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
/** The pending reopen of a stream that closed for good, and how many reopens
 * in a row have failed, which sets the wait before the next. */
let reconnectTimer: number | null = null;
let reconnectAttempts = 0;
/** Every collection read takes the next number when it is sent, and an answer
 * is published only when no later-sent read has already been. Each stream
 * event starts a read, so a burst of them can answer out of order: the older
 * answer arriving last would otherwise put back a name or a Vault the newer
 * one had already replaced (#646). Counts are sequenced the same way. */
let discoveryRequests = 0;
let newestDiscoveryPublished = 0;
let countRequests = 0;
let newestCountsPublished = 0;
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
  const request = (countRequests += 1);
  // Superseded: another generation, or a later-sent counts read already
  // published. Checked after every await.
  const stale = () =>
    forGeneration !== generation || request < newestCountsPublished;
  try {
    const res = await apiFetch("/api/v1/vaults/all/stats");
    if (stale()) {
      return;
    }
    if (!res.ok) {
      newestCountsPublished = request;
      publish({ noteCountsPartial: true });
      return;
    }
    const projection = (await res.json()) as VaultReadProjection<
      VaultStatistics[]
    >;
    if (stale()) {
      return;
    }
    newestCountsPublished = request;
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
    if (stale()) {
      return;
    }
    newestCountsPublished = request;
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
  const request = (discoveryRequests += 1);
  const stale = () =>
    forGeneration !== generation || request < newestDiscoveryPublished;
  try {
    const discovery = await readDiscovery();
    if (stale()) {
      return;
    }
    newestDiscoveryPublished = request;
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
    // A failure older than a published answer says nothing about it. A newer
    // failure does: an older answer arriving after it would put back what
    // the failed read was sent to replace, so it counts as published too.
    if (stale()) {
      return;
    }
    newestDiscoveryPublished = request;
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

/** The wait before reopening a stream that closed for good: 1 s, doubling
 * with each failed reopen, capped at 30 s. */
const STREAM_RETRY_BASE_MS = 1_000;
const STREAM_RETRY_MAX_MS = 30_000;

function cancelReconnect(): void {
  if (reconnectTimer !== null) {
    window.clearTimeout(reconnectTimer);
    reconnectTimer = null;
  }
}

function closeStream(): void {
  cancelReconnect();
  stream?.close();
  stream = null;
}

/** Reopen the stream after `delay`. The read that comes first goes through
 * `apiFetch`, so a server that now refuses the token raises the app's own
 * token prompt (an EventSource cannot see the status it was refused with). */
function scheduleReconnect(delay: number): void {
  if (reconnectTimer !== null) {
    return;
  }
  reconnectTimer = window.setTimeout(() => {
    reconnectTimer = null;
    reconnectAttempts += 1;
    openRevisionStream({ reopened: true });
    void loadCollection(generation);
  }, delay);
}

function openRevisionStream({ reopened = false } = {}): void {
  if (!("EventSource" in window)) {
    return;
  }
  // A fresh URL each time: the token may have changed since the last one.
  const events = new EventSource(withAccessToken("/api/v1/vaults/events"));
  stream = events;
  // The first stream's first open needs no read: `start` has just made one.
  // A reopened stream's does. The read sent with the reopen may have met the
  // proxy's 502 a moment before the server came back, and the stream's first
  // event may repeat the revision held before.
  let opened = reopened;
  events.addEventListener("open", () => {
    if (stream !== events) {
      return;
    }
    reconnectAttempts = 0;
    // The browser reconnects by itself after a dropped connection, and the
    // first event after it may repeat the revision held before, even though
    // a restarted server counted back up to it through different changes.
    if (opened) {
      void loadCollection(generation);
    }
    opened = true;
  });
  events.addEventListener("error", () => {
    if (stream !== events) {
      return;
    }
    // While `CONNECTING` the browser is retrying on its own. `CLOSED` means it
    // gave up for good: a reconnect answered with anything but 200, as a
    // reverse proxy's 502 during a restart or a 401 after a token change is.
    // Nothing else would ever reopen it, and the collection would stay as it
    // was until a reload (#646).
    if (events.readyState !== EventSource.CLOSED) {
      return;
    }
    events.close();
    stream = null;
    scheduleReconnect(
      Math.min(
        STREAM_RETRY_BASE_MS * 2 ** reconnectAttempts,
        STREAM_RETRY_MAX_MS,
      ),
    );
  });
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

/** Back on the tab, or back online: whatever happened meanwhile may have gone
 * unheard, so read the collection again, and reopen a closed stream now
 * rather than at the end of its backoff. */
function resume(): void {
  if (document.visibilityState === "hidden") {
    return;
  }
  if (stream === null) {
    cancelReconnect();
    reconnectAttempts = 0;
    openRevisionStream();
  }
  void loadCollection(generation);
}

function start(): void {
  started = true;
  openRevisionStream();
  document.addEventListener("visibilitychange", resume);
  window.addEventListener("online", resume);
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
  closeStream();
  reconnectAttempts = 0;
  document.removeEventListener("visibilitychange", resume);
  window.removeEventListener("online", resume);
  state = EMPTY_STATE;
  for (const listener of [...listeners]) {
    listener();
  }
  if (listeners.size > 0) {
    start();
  }
}
