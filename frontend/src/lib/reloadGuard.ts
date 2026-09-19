/**
 * A hold the editor takes so the app does not hard-reload itself over an
 * unsaved edit (#330).
 *
 * The service worker is registered in `autoUpdate` mode, so a fresh nightly
 * build installs, activates and reloads the page on its own, with no prompt,
 * triggered by a one-hour interval, tab focus, or the tab becoming visible.
 * That reload is invisible to React: it happens between keystrokes and takes
 * whatever has not reached the vault with it. The local draft now survives it,
 * but a reload the user did not ask for is still an interruption worth not
 * causing, so `main.tsx` asks here before pulling an update and before acting
 * on one that has already activated.
 *
 * A hold is named so two holders cannot cancel each other, and is released by
 * name; a component that unmounts while holding releases in its cleanup, which
 * is what stops a hold outliving the note it was taken for. Nothing here is
 * persisted: the question is only ever about this page, right now.
 */

const holders = new Set<string>();
const waiting = new Set<() => void>();

/** Take or release `holder`'s hold on reloading. */
export function holdAppReload(holder: string, held: boolean): void {
  if (held) {
    holders.add(holder);
    return;
  }
  if (!holders.delete(holder) || holders.size > 0) {
    return;
  }
  // Copied before running: a callback is free to take a new hold, and that
  // hold must not be dropped by the drain it arrived during.
  const ready = [...waiting];
  waiting.clear();
  for (const run of ready) {
    run();
  }
}

export function isAppReloadHeld(): boolean {
  return holders.size > 0;
}

/**
 * Run `action` now if nothing is holding, otherwise once the last hold is
 * released. A reload deferred this way waits as long as the edit does, which
 * is the point: the worker is already active, so the next navigation picks the
 * new build up anyway.
 */
export function whenAppReloadReleased(action: () => void): void {
  if (!isAppReloadHeld()) {
    action();
    return;
  }
  waiting.add(action);
}

/** Test seam: drop every hold and every deferred action. */
export function resetAppReloadHolds(): void {
  holders.clear();
  waiting.clear();
}
