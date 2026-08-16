import { useState } from "react";

import { apiFetch } from "../api/api";
import { readErrorMessage } from "../api/apiError";
import { UiButton } from "./ui";

/**
 * The confirmed recovery action offered only after a failed legacy import
 * (#150): safe because there was never a Vault list to lose. One
 * confirmation, one dialog, exactly the documented wording — notes and
 * history are untouched, old settings are ignored from now on, and the
 * folder must be added back by hand.
 */
export function StartWithNoVaultsDialog({
  onClose,
  onConfirmed,
}: {
  onClose: () => void;
  onConfirmed: () => void;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const confirm = async () => {
    setBusy(true);
    setError(null);
    try {
      const response = await apiFetch("/api/v1/vaults/start-with-no-vaults", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ confirm: true }),
      });
      if (!response.ok) {
        setError(
          await readErrorMessage(response, "Could not start with no Vaults"),
        );
        setBusy(false);
        return;
      }
      onConfirmed();
    } catch {
      setError("Could not start with no Vaults.");
      setBusy(false);
    }
  };

  return (
    <div
      className="modal-backdrop"
      role="presentation"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) {
          onClose();
        }
      }}
    >
      <section
        className="modal-panel"
        role="dialog"
        aria-modal="true"
        aria-label="Start with no Vaults"
      >
        <h2>Start with no Vaults</h2>
        <p>
          Notes and history are untouched, old settings will be ignored from now
          on, and the folder must be added by hand.
        </p>
        {error ? <p className="error">{error}</p> : null}
        <div className="modal-actions">
          <UiButton
            type="button"
            disabled={busy}
            onClick={() => void confirm()}
          >
            Start with no Vaults
          </UiButton>
          <UiButton
            type="button"
            className="close-note"
            disabled={busy}
            onClick={onClose}
          >
            Cancel
          </UiButton>
        </div>
      </section>
    </div>
  );
}
