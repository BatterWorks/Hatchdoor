// React inside CodeMirror widgets, through portals rather than separate
// roots. A widget's `toDOM` hands its empty container to the registry, and
// the editor component renders the reading view's own component into it
// with `createPortal`, so the diagram, the saved-query table and the PDF
// preview keep the page's router and saved-query contexts and are the same
// code the reading view draws. Nothing here writes: the widgets are views.

import type { ReactNode } from "react";

export type Mounted = { key: number; node: ReactNode };

export type PortalRegistry = {
  /** Draw `node` inside `host` until `unmount` is called for it. */
  mount: (host: HTMLElement, node: ReactNode) => void;
  unmount: (host: HTMLElement) => void;
  subscribe: (listener: () => void) => () => void;
  snapshot: () => ReadonlyMap<HTMLElement, Mounted>;
};

export function createPortalRegistry(): PortalRegistry {
  let hosts: ReadonlyMap<HTMLElement, Mounted> = new Map();
  let nextKey = 0;
  const listeners = new Set<() => void>();
  const notify = () => {
    for (const listener of listeners) {
      listener();
    }
  };
  return {
    mount: (host, node) => {
      // A host keeps its key across a re-mount, so the portal's own state
      // (a PDF's page, a table's sort) survives the widget being redrawn.
      const key = hosts.get(host)?.key ?? nextKey++;
      hosts = new Map(hosts).set(host, { key, node });
      notify();
    },
    unmount: (host) => {
      if (!hosts.has(host)) {
        return;
      }
      const next = new Map(hosts);
      next.delete(host);
      hosts = next;
      notify();
    },
    subscribe: (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    snapshot: () => hosts,
  };
}
