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
 * hot ink followed by a middot, never a `/`, so it reads as visibly not a
 * folder segment. Inert — a plain span, never a click target — and never
 * elides; callers give the adjacent title or path the shrinking room instead.
 */
export function VaultPrefix({ name }: { name: string }) {
  return (
    <span className="vault-prefix">
      {name}
      <span className="vault-prefix-dot" aria-hidden="true">
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
}: {
  title: string;
  description: string;
  actionLabel?: string;
  onAction?: () => void;
}) {
  return (
    <UiPanel className="state-block ui-empty-state">
      <h2>{title}</h2>
      <p>{description}</p>
      {actionLabel && onAction ? (
        <UiButton className="close-note" onClick={onAction}>
          {actionLabel}
        </UiButton>
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
