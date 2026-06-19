import { useState, type CSSProperties } from "react";

const backdrop: CSSProperties = {
  position: "fixed",
  inset: 0,
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
  background: "rgba(0, 0, 0, 0.6)",
  zIndex: 1000,
};

const panel: CSSProperties = {
  background: "var(--surface, #1e1e1e)",
  color: "var(--text, #eee)",
  padding: "1.5rem",
  borderRadius: "0.5rem",
  maxWidth: "22rem",
  width: "90%",
  display: "flex",
  flexDirection: "column",
  gap: "0.75rem",
  boxShadow: "0 8px 30px rgba(0, 0, 0, 0.4)",
};

const input: CSSProperties = {
  padding: "0.5rem",
  fontSize: "1rem",
  borderRadius: "0.25rem",
  border: "1px solid var(--border, #444)",
  background: "var(--bg, #111)",
  color: "inherit",
};

const button: CSSProperties = {
  padding: "0.5rem",
  fontSize: "1rem",
  borderRadius: "0.25rem",
  border: "none",
  background: "var(--accent, #3b82f6)",
  color: "#fff",
  cursor: "pointer",
};

/** Shown when the API returns 401, prompting for the web bearer token. */
export function TokenPrompt({ onSubmit }: { onSubmit: (token: string) => void }) {
  const [value, setValue] = useState("");

  return (
    <div
      style={backdrop}
      role="dialog"
      aria-modal="true"
      aria-label="Access token required"
    >
      <form
        style={panel}
        onSubmit={(event) => {
          event.preventDefault();
          const trimmed = value.trim();
          if (trimmed) {
            onSubmit(trimmed);
          }
        }}
      >
        <h2 style={{ margin: 0, fontSize: "1.1rem" }}>Access token required</h2>
        <p style={{ margin: 0, fontSize: "0.9rem" }}>
          This Hatchdoor instance requires an access token to read the vault.
        </p>
        <input
          style={input}
          type="password"
          autoFocus
          value={value}
          onChange={(event) => setValue(event.target.value)}
          placeholder="Bearer token"
          aria-label="Access token"
        />
        <button style={button} type="submit">
          Unlock
        </button>
      </form>
    </div>
  );
}
