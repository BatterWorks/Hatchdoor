/**
 * PROTOTYPE — throwaway route for issue #58: three structurally different
 * Settings pages, switchable via `?variant=`, mounted inside the real app shell
 * at /settings-prototype. Delete this directory once a variant wins.
 *
 * All data is fixture data (see fixtures.ts) — nothing here touches the server.
 * `?scenario=` swaps the fixture so the awkward states can be judged: a fully
 * .env-pinned instance, local-only versioning, a rebuild in progress.
 */

import { useEffect } from "react";
import { useSearchParams } from "react-router-dom";

import { SCENARIOS, type Scenario } from "./fixtures";
import "./settings-prototype.css";
import { useSandbox } from "./useSandbox";
import { NAME as NAME_A, VariantA } from "./VariantA";
import { NAME as NAME_B, VariantB } from "./VariantB";
import { NAME as NAME_C, VariantC } from "./VariantC";

const VARIANTS = [
  { id: "A", name: NAME_A },
  { id: "B", name: NAME_B },
  { id: "C", name: NAME_C },
];

export function SettingsPrototypePage() {
  const [params, setParams] = useSearchParams();
  const variant = (params.get("variant") ?? "A").toUpperCase();
  const scenario = (params.get("scenario") ?? "mixed") as Scenario;
  const sb = useSandbox(scenario);

  const setVariant = (next: string) => {
    const p = new URLSearchParams(params);
    p.set("variant", next);
    setParams(p, { replace: true });
  };

  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      const el = document.activeElement;
      if (
        el instanceof HTMLElement &&
        (el.tagName === "INPUT" ||
          el.tagName === "TEXTAREA" ||
          el.isContentEditable ||
          el.tagName === "SELECT")
      ) {
        return;
      }
      if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
      const i = VARIANTS.findIndex((v) => v.id === variant);
      const step = event.key === "ArrowRight" ? 1 : -1;
      setVariant(VARIANTS[(i + step + VARIANTS.length) % VARIANTS.length].id);
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  const current = VARIANTS.find((v) => v.id === variant) ?? VARIANTS[0];

  return (
    <div className="sp-root">
      {variant === "B" ? (
        <VariantB sb={sb} />
      ) : variant === "C" ? (
        <VariantC sb={sb} />
      ) : (
        <VariantA sb={sb} />
      )}

      {import.meta.env.DEV ? (
        <div className="sp-switcher">
          <button
            className="sp-switcher-arrow"
            aria-label="Previous variant"
            onClick={() => {
              const i = VARIANTS.findIndex((v) => v.id === current.id);
              setVariant(
                VARIANTS[(i - 1 + VARIANTS.length) % VARIANTS.length].id,
              );
            }}
          >
            ←
          </button>
          <span className="sp-switcher-label">
            {current.id} — {current.name}
          </span>
          <button
            className="sp-switcher-arrow"
            aria-label="Next variant"
            onClick={() => {
              const i = VARIANTS.findIndex((v) => v.id === current.id);
              setVariant(VARIANTS[(i + 1) % VARIANTS.length].id);
            }}
          >
            →
          </button>
          <span className="sp-switcher-sep" />
          <select
            className="sp-switcher-select"
            value={scenario}
            aria-label="Scenario"
            onChange={(e) => {
              const p = new URLSearchParams(params);
              p.set("scenario", e.target.value);
              setParams(p, { replace: true });
            }}
          >
            {SCENARIOS.map((s) => (
              <option key={s.id} value={s.id}>
                {s.label}
              </option>
            ))}
          </select>
          <span className="sp-switcher-blurb">
            {SCENARIOS.find((s) => s.id === scenario)?.blurb}
          </span>
        </div>
      ) : null}
    </div>
  );
}
