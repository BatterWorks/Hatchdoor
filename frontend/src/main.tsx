import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { BrowserRouter } from "react-router-dom";
import { registerSW } from "virtual:pwa-register";
import "katex/dist/katex.min.css";
import "./index.css";
import App from "./App";

const SW_UPDATE_INTERVAL_MS = 60 * 60 * 1000;

registerSW({
  immediate: true,
  onRegisteredSW(_swUrl, registration) {
    if (!registration) {
      return;
    }

    const update = () => {
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
  onNeedRefresh() {
    window.location.reload();
  },
});

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <BrowserRouter>
      <App />
    </BrowserRouter>
  </StrictMode>,
);
