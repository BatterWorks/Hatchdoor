import { useEffect, useRef, type ReactNode } from "react";
import { createPortal } from "react-dom";

import { useHelp } from "../help";

const FOCUSABLE_SELECTOR =
  'a[href], button:not([disabled]), textarea:not([disabled]), input:not([disabled]), select:not([disabled]), [tabindex]:not([tabindex="-1"])';

/** The one modal shell every Settings dialog renders through (#338): the Vault
 * creation flow, the identity-change confirmation, and the reindex
 * confirmation. It gives them what the app's other dialogs already had
 * (`NoteActionsDialog`): focus moves into the dialog when it opens, Tab and
 * Shift+Tab stay inside it, Escape closes it, and focus returns to whatever
 * opened it. `aria-modal` tells assistive tech the rest of the page is inert,
 * so keyboard focus must not be able to walk out into it.
 *
 * Help can open beside it from a link inside (#430). While it is open, the
 * dialog makes room for it, Escape closes Help first, and Tab is not held
 * inside the dialog, since Help is not modal: the What's new dialog's rules.
 *
 * `closeDisabled` holds Escape while the dialog is doing something its own
 * Cancel button is disabled for. A backdrop click deliberately does not
 * close: the creation form holds typed input a stray click would lose.
 *
 * The backdrop mounts on `document.body` (#448). Add a Vault opens from the
 * Settings index, which is `position: sticky` and so a stacking context of
 * its own: rendered in place, the dialog stacked only within the sidebar and
 * the main column's buttons painted over it. */
export function SettingsModal({
  label,
  onClose,
  closeDisabled = false,
  className,
  children,
}: {
  label: string;
  onClose: () => void;
  closeDisabled?: boolean;
  className?: string;
  children: ReactNode;
}) {
  const dialogRef = useRef<HTMLDivElement>(null);
  const help = useHelp();
  const helpRef = useRef(help);
  // Read through refs so a parent re-render (a new `onClose` closure every
  // time) never re-runs the effect below and steals focus back to the first
  // control mid-typing.
  const onCloseRef = useRef(onClose);
  const closeDisabledRef = useRef(closeDisabled);
  useEffect(() => {
    onCloseRef.current = onClose;
    closeDisabledRef.current = closeDisabled;
    helpRef.current = help;
  });

  useEffect(() => {
    const dialog = dialogRef.current;
    if (!dialog) return;
    const opener =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
    const focusable = () =>
      Array.from(dialog.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR));

    (focusable()[0] ?? dialog).focus();

    // On the document rather than the dialog, so a key pressed while focus
    // has somehow left the dialog (a click on the backdrop) is still caught
    // and brought back.
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        if (helpRef.current.isOpen) helpRef.current.closeHelp();
        else if (!closeDisabledRef.current) onCloseRef.current();
        return;
      }
      if (event.key !== "Tab" || helpRef.current.isOpen) return;
      const items = focusable();
      if (items.length === 0) {
        event.preventDefault();
        dialog.focus();
        return;
      }
      const first = items[0];
      const last = items[items.length - 1];
      const active = document.activeElement;
      if (!(active instanceof Node) || !dialog.contains(active)) {
        event.preventDefault();
        (event.shiftKey ? last : first).focus();
      } else if (event.shiftKey && active === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && active === last) {
        event.preventDefault();
        first.focus();
      }
    };

    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("keydown", onKeyDown);
      if (opener?.isConnected) opener.focus();
    };
  }, []);

  return createPortal(
    <div
      className={`settings-modal-back${help.isOpen ? " is-beside-help" : ""}`}
      role="presentation"
    >
      <div
        ref={dialogRef}
        className={className ? `settings-modal ${className}` : "settings-modal"}
        role="dialog"
        aria-modal="true"
        aria-label={label}
        tabIndex={-1}
      >
        {children}
      </div>
    </div>,
    document.body,
  );
}
