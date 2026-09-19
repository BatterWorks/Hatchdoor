import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { BrowserRouter } from "react-router-dom";
import { registerSW } from "virtual:pwa-register";
import "katex/dist/katex.min.css";
import "./index.css";
import App from "./App";
import { isAppReloadHeld, whenAppReloadReleased } from "./lib/reloadGuard";
import { clearLegacyNoteScopedBrowserState } from "./lib/storage";
import { collectLegacyHeldDrafts } from "./lib/writeDrafts";

const SW_UPDATE_INTERVAL_MS = 60 * 60 * 1000;

// Run once, synchronously, before the tree ever renders (#151): every
// component's first read of browser state and held drafts must already
// reflect the migration, regardless of which route mounts first.
collectLegacyHeldDrafts();
clearLegacyNoteScopedBrowserState();

registerSW({
  immediate: true,
  onRegisteredSW(_swUrl, registration) {
    if (!registration) {
      return;
    }

    const update = () => {
      // An update found now is an update activated now: the worker calls
      // `skipWaiting`/`clientsClaim`, so checking mid-edit is what schedules
      // the reload (#330). Coming back to the tab is one of the triggers, and
      // that is exactly the moment an unsaved block is sitting open.
      if (isAppReloadHeld()) {
        return;
      }
      void registration.update();
    };

    window.setInterval(update, SW_UPDATE_INTERVAL_MS);
    document.addEventListener("visibilitychange", () => {
      if (document.visibilityState === "visible") {
        update();
      }
    });
    window.addEventListener("focus", update);
  },
  // `autoUpdate` reloads the page itself the moment a new worker activates,
  // unless this hook takes the decision over. It does, so the reload waits for
  // the editor to let go (#330): a worker discovered by another tab, or
  // installed just before the hold was taken, still gets here.
  onNeedReload() {
    whenAppReloadReleased(() => window.location.reload());
  },
});

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <BrowserRouter>
      <App />
    </BrowserRouter>
  </StrictMode>,
);
