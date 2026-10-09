// The portals the widget registry asks for, rendered by the editor
// component so they sit under the page's router and saved-query contexts.

import { useSyncExternalStore } from "react";
import { createPortal } from "react-dom";

import type { PortalRegistry } from "./widgetPortals";

export function WidgetPortals({ registry }: { registry: PortalRegistry }) {
  const hosts = useSyncExternalStore(registry.subscribe, registry.snapshot);
  return (
    <>
      {Array.from(hosts, ([host, { key, node }]) =>
        createPortal(node, host, `widget-${key}`),
      )}
    </>
  );
}
