import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type KeyboardEvent,
} from "react";
import ReactMarkdown, { type Components } from "react-markdown";

import { UiButton } from "../../components/ui";
import { useHelp } from "../help";
import {
  baseVersion,
  carriesUsageReportSwitch,
  fetchUsageReportSetting,
  fetchWhatsNew,
  markSeen,
  readSeen,
  releasesToShow,
  saveUsageReport,
  type Highlight,
  type HighlightLink,
  type Release,
  type UsageReportSetting,
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
  const [usageReport, setUsageReport] = useState<UsageReportSetting | null>(
    null,
  );
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

  // The usage report switch (ADR-48) needs the setting as it stands. Read
  // only when a line on screen carries the switch; without an answer the
  // line shows as any other.
  const offersUsageReport =
    notice?.releases.some((release) =>
      release.highlights.some((highlight) =>
        carriesUsageReportSwitch(release.version, highlight),
      ),
    ) ?? false;
  useEffect(() => {
    if (!offersUsageReport) {
      return;
    }
    const controller = new AbortController();
    void fetchUsageReportSetting(controller.signal).then((setting) => {
      if (!controller.signal.aborted) {
        setUsageReport(setting);
      }
    });
    return () => controller.abort();
  }, [offersUsageReport]);

  // A layout effect, so the dialog holds focus before it is painted. A
  // passive effect runs after, which leaves it on screen without focus for a
  // moment (#468).
  useLayoutEffect(() => {
    if (notice) {
      dialogRef.current?.focus();
    }
  }, [notice]);

  // Tab stays inside the dialog, as in the app's other modals, except while
  // Help is open beside it: Help is not modal, so focus may move into it.
  // On the document, so a Tab after focus left the dialog brings it back.
  // A layout effect for the same reason as the focus above: the trap is in
  // place before the dialog is painted.
  const helpOpen = help.isOpen;
  useLayoutEffect(() => {
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
                      usageReport={
                        carriesUsageReportSwitch(release.version, highlight)
                          ? usageReport
                          : null
                      }
                      onUsageReportChange={setUsageReport}
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
  usageReport,
  onUsageReportChange,
}: {
  highlight: Highlight;
  /** Set on a pinned item, which names the release it came from. */
  version?: string;
  onOpenLink: (link: HighlightLink) => void;
  /** Set on the one line that carries the usage report switch. */
  usageReport?: UsageReportSetting | null;
  onUsageReportChange?: (setting: UsageReportSetting) => void;
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
      {usageReport && onUsageReportChange ? (
        <UsageReportSwitch
          setting={usageReport}
          onChange={onUsageReportChange}
        />
      ) : null}
    </li>
  );
}

/** The usage report switch on its What's new line (ADR-48): the setting as
 * it stands, changed only by a click. A value the server's configuration
 * file holds is reported and cannot be changed here. */
function UsageReportSwitch({
  setting,
  onChange,
}: {
  setting: UsageReportSetting;
  onChange: (setting: UsageReportSetting) => void;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Not `disabled` while saving: a disabled button drops focus, and the
  // dialog's focus trap would then move it elsewhere.
  const toggle = async () => {
    if (busy) {
      return;
    }
    setBusy(true);
    setError(null);
    const saved = await saveUsageReport(!setting.on);
    setBusy(false);
    if (saved.ok) {
      onChange(saved.setting);
    } else {
      setError(saved.message);
    }
  };
  return (
    <div className="whats-new-switch">
      <span className="whats-new-switch-label">Send a usage report</span>
      {setting.locked ? (
        <span className="whats-new-switch-locked">
          {setting.on ? "On" : "Off"}, set in the server&apos;s configuration
          file
        </span>
      ) : (
        <button
          type="button"
          className="settings-toggle"
          aria-label="Send a usage report"
          aria-pressed={setting.on}
          aria-busy={busy}
          onClick={() => void toggle()}
        >
          <span className="settings-toggle-track">
            <span className="settings-toggle-knob" />
          </span>
          <span>{setting.on ? "On" : "Off"}</span>
        </button>
      )}
      {error ? (
        <p className="whats-new-switch-error" role="alert">
          {error}
        </p>
      ) : null}
    </div>
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
