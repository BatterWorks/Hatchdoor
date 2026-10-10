import { useState } from "react";

import { HELP_PAGES, useHelp } from "../features/help";
import { UiButton } from "./ui";

/** Shown when the API returns 401, prompting for the web bearer token. */
export function TokenPrompt({
  onSubmit,
}: {
  onSubmit: (token: string) => void;
}) {
  const [value, setValue] = useState("");
  const { openHelp } = useHelp();

  return (
    <div
      className="modal-backdrop token-prompt"
      role="dialog"
      aria-modal="true"
      aria-label="Access token required"
    >
      <div className="modal-panel">
        <h2>Access token required</h2>
        <form
          onSubmit={(event) => {
            event.preventDefault();
            const trimmed = value.trim();
            if (trimmed) {
              onSubmit(trimmed);
            }
          }}
        >
          <p className="token-prompt-lede">
            This Hatchdoor instance requires an access token to read the vault.
          </p>
          <input
            className="field-input"
            type="password"
            autoFocus
            value={value}
            onChange={(event) => setValue(event.target.value)}
            placeholder="Bearer token"
            aria-label="Access token"
          />
          <div className="modal-actions">
            <UiButton type="submit">Unlock</UiButton>
          </div>
          <p className="token-prompt-help">
            <button
              type="button"
              className="help-link"
              onClick={() =>
                openHelp(HELP_PAGES.install, "where-do-i-find-my-token")
              }
            >
              Where do I find my token?
            </button>
            <button
              type="button"
              className="help-link"
              onClick={() => openHelp()}
            >
              Help
            </button>
          </p>
        </form>
      </div>
    </div>
  );
}
