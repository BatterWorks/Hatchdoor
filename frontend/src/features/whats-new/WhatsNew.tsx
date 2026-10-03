import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import ReactMarkdown, { type Components } from "react-markdown";

import { UiButton } from "../../components/ui";
import { useHelp } from "../help";
import {
  baseVersion,
  fetchWhatsNew,
  markSeen,
  readSeen,
  releasesToShow,
  type Highlight,
  type HighlightLink,
  type Release,
} from "./whatsNew";

const FOCUSABLE_SELECTOR =
  'a[href], button:not([disabled]), textarea:not([disabled]), input:not([disabled]), select:not([disabled]), [tabindex]:not([tabindex="-1"])';

/** The What's new page in the manual, for "Full changelog". */
const WHATS_NEW_PAGE = "whats-new";

/** A highlight is one line: inline Markdown only, links leave in a new tab. */
const INLINE_ELEMENTS = ["strong", "em", "code", "del", "a"];
const INLINE_COMPONENTS: Components = {
  a: ({ href, children }) => (
    <a href={href} target="_blank" rel="noopener noreferrer">
      {children}
    </a>
  ),
};

/** What the dialog shows: the running version and the releases it missed. */
type PendingNotice = { version: string; releases: Release[] };

/**
 * The What's new pop-up (#418): after an upgrade, a centred dialog with the
 * highlights of every release since this browser last looked, action-needed
 * items pinned on top. "Got it" marks the running version as seen here. The
 * shell mounts it signed in and never in demo mode; it shows nothing on a
 * fresh install, when nothing is new, or when storage is blocked.
 */
export function WhatsNew() {
  const [notice, setNotice] = useState<PendingNotice | null>(null);
  const help = useHelp();
  const dialogRef = useRef<HTMLElement>(null);

  useEffect(() => {
    const record = readSeen();
    if (!record.available) {
      return;
    }
    const controller = new AbortController();
    void fetchWhatsNew(controller.signal).then((response) => {
      if (!response || controller.signal.aborted) {
        return;
      }
      const releases = releasesToShow(response, record);
      if (releases.length > 0) {
        setNotice({ version: response.version, releases });
      }
    });
    return () => controller.abort();
  }, []);

  useEffect(() => {
    if (notice) {
      dialogRef.current?.focus();
    }
  }, [notice]);

  // Tab stays inside the dialog, as in the app's other modals, except while
  // Help is open beside it: Help is not modal, so focus may move into it.
  // On the document, so a Tab after focus left the dialog brings it back.
  const helpOpen = help.isOpen;
  useEffect(() => {
    const dialog = dialogRef.current;
    if (!notice || helpOpen || !dialog) {
      return;
    }
    const onKeyDown = (event: globalThis.KeyboardEvent) => {
      if (event.key !== "Tab") {
        return;
      }
      const items = Array.from(
        dialog.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR),
      );
      const first = items[0];
      const last = items[items.length - 1];
      const active = document.activeElement;
      if (!first || !last) {
        event.preventDefault();
        dialog.focus();
      } else if (!(active instanceof Node) || !dialog.contains(active)) {
        event.preventDefault();
        (event.shiftKey ? last : first).focus();
      } else if (event.shiftKey && (active === first || active === dialog)) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && active === last) {
        event.preventDefault();
        first.focus();
      }
    };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [notice, helpOpen]);

  if (!notice) {
    return null;
  }

  const dismiss = () => {
    markSeen(notice.version);
    setNotice(null);
  };

  const openLink = (link: HighlightLink) => {
    help.openHelp(link.page, link.heading ?? undefined);
  };

  const onKeyDown = (event: KeyboardEvent) => {
    if (event.key !== "Escape") {
      return;
    }
    event.preventDefault();
    if (help.isOpen) {
      help.closeHelp();
    } else {
      dismiss();
    }
  };

  const running = baseVersion(notice.version);
  const skipped = notice.releases.length - 1;
  const pinned = notice.releases.flatMap((release) =>
    release.highlights
      .filter((highlight) => highlight.action_needed)
      .map((highlight) => ({ version: release.version, highlight })),
  );

  return (
    <div
      className={`modal-backdrop whats-new-backdrop${help.isOpen ? " is-beside-help" : ""}`}
      role="presentation"
    >
      <section
        ref={dialogRef}
        className="modal-panel whats-new"
        role="dialog"
        aria-modal="true"
        aria-labelledby="whats-new-title"
        tabIndex={-1}
        onKeyDown={onKeyDown}
      >
        <header className="whats-new-head">
          <span className="help-eyebrow">What&apos;s new</span>
          <h2 id="whats-new-title" className="whats-new-title">
            Hatchdoor {running}
          </h2>
          <p className="whats-new-sub">
            Updated to {running}.
            {skipped > 0
              ? ` You skipped ${skipped} release${skipped === 1 ? "" : "s"} since you last looked.`
              : null}
          </p>
        </header>
        <div className="whats-new-body">
          {pinned.length > 0 ? (
            <section className="whats-new-action" aria-label="Action needed">
              <p className="whats-new-action-label">Action needed</p>
              <ul className="whats-new-list">
                {pinned.map(({ version, highlight }, index) => (
                  <HighlightItem
                    key={`${version}-${index}`}
                    highlight={highlight}
                    version={version}
                    onOpenLink={openLink}
                  />
                ))}
              </ul>
            </section>
          ) : null}
          {notice.releases.map((release) => {
            const otherHighlights = release.highlights.filter(
              (h) => !h.action_needed,
            );
            if (otherHighlights.length === 0) {
              return null;
            }
            return (
              <section
                key={release.version}
                className="whats-new-release"
                aria-label={`Version ${release.version}`}
              >
                <h3 className="whats-new-release-head">
                  <span>v{release.version}</span>
                  <time className="whats-new-date" dateTime={release.date}>
                    {formatDate(release.date)}
                  </time>
                </h3>
                <ul className="whats-new-list">
                  {otherHighlights.map((highlight, index) => (
                    <HighlightItem
                      key={index}
                      highlight={highlight}
                      onOpenLink={openLink}
                    />
                  ))}
                </ul>
              </section>
            );
          })}
        </div>
        <footer className="whats-new-foot">
          <button
            type="button"
            className="help-link whats-new-link"
            onClick={() => help.openHelp(WHATS_NEW_PAGE)}
          >
            Full changelog
          </button>
          <UiButton type="button" onClick={dismiss}>
            Got it
          </UiButton>
        </footer>
      </section>
    </div>
  );
}

function HighlightItem({
  highlight,
  version,
  onOpenLink,
}: {
  highlight: Highlight;
  /** Set on a pinned item, which names the release it came from. */
  version?: string;
  onOpenLink: (link: HighlightLink) => void;
}) {
  const { link } = highlight;
  return (
    <li>
      {version ? (
        <span className="whats-new-version-tag">v{version} </span>
      ) : null}
      <ReactMarkdown
        allowedElements={INLINE_ELEMENTS}
        unwrapDisallowed
        components={INLINE_COMPONENTS}
      >
        {highlight.text}
      </ReactMarkdown>
      {link ? (
        <>
          {" "}
          <button
            type="button"
            className="help-link whats-new-link"
            onClick={() => onOpenLink(link)}
          >
            {link.label}
          </button>
        </>
      ) : null}
    </li>
  );
}

/** `2026-10-20` → `20 Oct 2026` in the reader's locale; the raw date when it
 * does not parse. */
function formatDate(date: string): string {
  const parsed = new Date(`${date}T00:00:00Z`);
  if (Number.isNaN(parsed.getTime())) {
    return date;
  }
  return parsed.toLocaleDateString(undefined, {
    day: "numeric",
    month: "short",
    year: "numeric",
    timeZone: "UTC",
  });
}
