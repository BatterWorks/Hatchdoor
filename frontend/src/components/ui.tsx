import type { ButtonHTMLAttributes, HTMLAttributes, ReactNode } from "react";

export function UiButton({
  className,
  children,
  ...props
}: ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button className={`ui-button ${className ?? ""}`.trim()} {...props}>
      {children}
    </button>
  );
}

export function UiPanel({
  className,
  children,
  ...props
}: HTMLAttributes<HTMLElement>) {
  return (
    <section className={`ui-panel ${className ?? ""}`.trim()} {...props}>
      {children}
    </section>
  );
}

export function UiToolbar({
  className,
  children,
}: {
  className?: string;
  children: ReactNode;
}) {
  return (
    <div className={`ui-toolbar ${className ?? ""}`.trim()}>{children}</div>
  );
}

/**
 * A Vault declaring itself as a marked path root (#140): the Vault name in
 * muted mono followed by a middot, never a `/`, so it reads as visibly not a
 * folder segment. Inert — a plain span, never a click target. It takes at
 * most a share of its row and elides past that (#530): a long Vault name
 * used to push the note title off the row entirely, and the title is what
 * the row is for.
 */
export function VaultPrefix({ name }: { name: string }) {
  return (
    <span className="path-vault">
      {name}
      <span className="path-sep" aria-hidden="true">
        ·
      </span>
    </span>
  );
}

export function StatusBadge({
  tone,
  text,
}: {
  tone: "warn" | "error";
  text: string;
}) {
  return <span className={`ui-badge status-badge ${tone}`}>{text}</span>;
}

export function StateBlock({
  title,
  description,
  actionLabel,
  onAction,
  secondaryActionLabel,
  onSecondaryAction,
  tone,
  help,
}: {
  title: string;
  description: string;
  actionLabel?: string;
  onAction?: () => void;
  /** A second, non-primary action alongside `actionLabel` — e.g. a broken
   * start's `Try again` plus a confirmed recovery action (#150). Only
   * offered together with `actionLabel`/`onAction`; never alone. */
  secondaryActionLabel?: string;
  onSecondaryAction?: () => void;
  /** The documented §23 error variant (red heading) — a genuine failure,
   * never the plain empty shell used for "nothing here yet". */
  tone?: "error";
  /** A link to the manual page that explains this state (#423), on its own
   * line under the description. */
  help?: ReactNode;
}) {
  return (
    <UiPanel
      className={`state-block ui-empty-state${tone === "error" ? " error" : ""}`}
    >
      <h2>{title}</h2>
      <p>{description}</p>
      {help ? <p>{help}</p> : null}
      {actionLabel && onAction ? (
        secondaryActionLabel && onSecondaryAction ? (
          <div className="modal-actions">
            <UiButton className="close-note" onClick={onAction}>
              {actionLabel}
            </UiButton>
            <UiButton onClick={onSecondaryAction}>
              {secondaryActionLabel}
            </UiButton>
          </div>
        ) : (
          <UiButton className="close-note" onClick={onAction}>
            {actionLabel}
          </UiButton>
        )
      ) : null}
    </UiPanel>
  );
}

export function ExplorerSkeleton() {
  return (
    <div className="skeleton-list" aria-hidden="true">
      {Array.from({ length: 8 }).map((_, idx) => (
        <div
          key={idx}
          className="skeleton-line"
          style={{ width: `${72 - idx * 5}%` }}
        />
      ))}
    </div>
  );
}

export function NoteSkeleton() {
  return (
    <div className="skeleton-list" aria-hidden="true">
      <div className="skeleton-line" style={{ width: "45%" }} />
      <div className="skeleton-line" style={{ width: "90%" }} />
      <div className="skeleton-line" style={{ width: "84%" }} />
      <div className="skeleton-line" style={{ width: "88%" }} />
      <div className="skeleton-line" style={{ width: "72%" }} />
    </div>
  );
}
