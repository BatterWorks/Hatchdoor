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
 * `revision` is the collection revision the SSE stream last reported; it starts
 * at 0, meaning "nothing has changed since load".
 */
export type VaultCollectionState = {
  vaults: VaultSummary[];
  allVaults: VaultSummary[];
  demoMode: boolean;
  loading: boolean;
  error: string | null;
  recovery: VaultRegistryRecovery | null;
  legacyMigrationRecovery: LegacyMigrationRecovery | null;
  registryRevision: number | null;
  revision: number;
  noteCounts: Record<VaultId, number>;
};

const EMPTY_STATE: VaultCollectionState = {
  vaults: [],
  allVaults: [],
  demoMode: false,
  loading: true,
  error: null,
  recovery: null,
  legacyMigrationRecovery: null,
  registryRevision: null,
  revision: 0,
  noteCounts: {},
};

type Listener = () => void;

let state: VaultCollectionState = EMPTY_STATE;
const listeners = new Set<Listener>();
let started = false;
let stream: EventSource | null = null;
/** Bumped by every reset so a fetch still in flight over the old collection
 * cannot write its answer into the new one. */
let generation = 0;

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
  const next = { ...state, ...patch };
  const changed = (Object.keys(patch) as (keyof VaultCollectionState)[]).some(
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
    if (!res.ok || forGeneration !== generation) {
      return;
    }
    const projection = (await res.json()) as VaultReadProjection<
      VaultStatistics[]
    >;
    if (forGeneration !== generation) {
      return;
    }
    const next: Record<VaultId, number> = {};
    for (const entry of projection.data) {
      next[entry.vault_id] = entry.note_count;
    }
    publish({ noteCounts: reuseIfUnchanged(state.noteCounts, next) });
  } catch {
    // Leave prior counts in place; the slot treats a missing entry as unknown.
  }
}

/** The one `GET /api/v1/vaults` read in the app. Throws the server's own
 * message on a refusal, so each caller decides what to do with it. */
async function readDiscovery(): Promise<VaultDiscoveryResponse> {
  const res = await apiFetch("/api/v1/vaults");
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
      error: null,
    });
    // A broken registry has no collection to count, and the stats read would
    // only produce a second error saying the same thing. Neither has an empty
    // browsing list: the `all` scope those counts come from covers enabled
    // Vaults, and every reader of them renders one.
    if (recovery || enabled.length === 0) {
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
