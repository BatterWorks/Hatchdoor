import {
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type MouseEvent,
  type ReactNode,
} from "react";
import ReactMarkdown from "react-markdown";
import rehypeKatex from "rehype-katex";
import remarkGfm from "remark-gfm";
import remarkMath from "remark-math";

import {
  ArrowBackIcon,
  CloseFullscreenIcon,
  CloseIcon,
  HomeIcon,
  OpenInFullIcon,
} from "../../components/icons";
import { createManualMarkdownComponents } from "../../components/note-page/renderers";
import { extractMarkdownHeadings } from "../../lib/noteHeadings";
import {
  HELP_HOME,
  HELP_PAGES,
  fetchHelpPage,
  helpPageTitle,
  plainExcerpt,
  resolveHelpLink,
  searchHelp,
  type HelpLocation,
  type HelpPageResult,
  type HelpSearchHit,
} from "./helpPages";

const REMARK_PLUGINS = [remarkGfm, remarkMath];
const REHYPE_PLUGINS = [rehypeKatex];
const SEARCH_DELAY_MS = 200;

/** Heading ids inside Help carry this prefix, so they never collide with the
 * ids of the note open underneath. */
const HEADING_ID_PREFIX = "help-";

type PageState = { page: string } & (HelpPageResult | { kind: "loading" });

type SearchState =
  | { kind: "idle" }
  | { kind: "loading"; query: string }
  | { kind: "done"; query: string; hits: HelpSearchHit[] }
  | { kind: "error"; query: string };

export function HelpPanel({
  location,
  canGoBack,
  fullWidth,
  demoMode,
  aboveDialogs,
  onOpenSetupChecklist,
  onNavigate,
  onBack,
  onHome,
  onToggleFullWidth,
  onClose,
}: {
  location: HelpLocation & { visit: number };
  canGoBack: boolean;
  fullWidth: boolean;
  demoMode: boolean;
  aboveDialogs: boolean;
  onOpenSetupChecklist?: () => void;
  onNavigate: (location: HelpLocation) => void;
  onBack: () => void;
  onHome: () => void;
  onToggleFullWidth: () => void;
  onClose: () => void;
}) {
  const panelRef = useRef<HTMLElement>(null);
  const bodyRef = useRef<HTMLDivElement>(null);
  const [top, setTop] = useState(0);
  const [query, setQuery] = useState("");
  const [pageState, setPageState] = useState<PageState>({
    page: location.page,
    kind: "loading",
  });
  const [search, setSearch] = useState<SearchState>({ kind: "idle" });
  const trimmedQuery = query.trim();
  const searching = trimmedQuery !== "";

  // Help starts below the top bar, so the bar stays usable beside it.
  useLayoutEffect(() => {
    const measure = () => {
      const bar = document.querySelector(".app-topbar");
      setTop(bar ? Math.max(0, bar.getBoundingClientRect().bottom) : 0);
    };
    measure();
    window.addEventListener("resize", measure);
    return () => window.removeEventListener("resize", measure);
  }, []);

  useEffect(() => {
    panelRef.current?.focus();
  }, []);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || event.defaultPrevented) {
        return;
      }
      // Another dialog open on top (search, a confirm) owns its own Escape.
      const target = event.target;
      const dialog =
        target instanceof Element ? target.closest('[role="dialog"]') : null;
      if (dialog && !panelRef.current?.contains(dialog)) {
        return;
      }
      event.preventDefault();
      onClose();
    };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  useEffect(() => {
    const controller = new AbortController();
    setPageState({ page: location.page, kind: "loading" });
    fetchHelpPage(location.page, controller.signal).then(
      (result) => setPageState({ page: location.page, ...result }),
      () => {},
    );
    return () => controller.abort();
  }, [location.page]);

  // A new visit leaves search and lands on its heading, or the page's top.
  useEffect(() => {
    setQuery("");
  }, [location.visit]);

  const pageReady = pageState.kind === "page";
  useEffect(() => {
    if (!pageReady || searching) {
      return;
    }
    const body = bodyRef.current;
    const id = location.heading ? HEADING_ID_PREFIX + location.heading : null;
    const heading = id
      ? Array.from(body?.querySelectorAll<HTMLElement>("[id]") ?? []).find(
          (element) => element.id === id,
        )
      : undefined;
    if (heading) {
      heading.scrollIntoView({ block: "start" });
    } else {
      body?.scrollTo?.({ top: 0 });
    }
  }, [pageReady, location.visit, location.heading, searching]);

  useEffect(() => {
    if (!searching) {
      setSearch({ kind: "idle" });
      return;
    }
    const controller = new AbortController();
    setSearch({ kind: "loading", query: trimmedQuery });
    const timer = window.setTimeout(() => {
      searchHelp(trimmedQuery, controller.signal).then(
        (hits) => setSearch({ kind: "done", query: trimmedQuery, hits }),
        () => {
          if (!controller.signal.aborted) {
            setSearch({ kind: "error", query: trimmedQuery });
          }
        },
      );
    }, SEARCH_DELAY_MS);
    return () => {
      window.clearTimeout(timer);
      controller.abort();
    };
  }, [searching, trimmedQuery]);

  const markdown = pageState.kind === "page" ? pageState.markdown : "";
  const title =
    location.page === HELP_HOME
      ? "Home"
      : helpPageTitle(markdown, pageState.kind === "loading" ? "" : "Help");

  const components = useMemo(() => {
    const headingIds = new Map(
      extractMarkdownHeadings(markdown).map(({ sourceLine, id }) => [
        sourceLine,
        HEADING_ID_PREFIX + id,
      ]),
    );
    const renderLink = (href: string | undefined, children: ReactNode) => {
      const target = resolveHelpLink(href, location.page);
      if (!target) {
        return (
          <a href={href} target="_blank" rel="noopener noreferrer">
            {children}
          </a>
        );
      }
      const address = `/docs/${target.page}.md${target.heading ? `#${target.heading}` : ""}`;
      return (
        <a
          href={address}
          onClick={(event: MouseEvent<HTMLAnchorElement>) => {
            if (
              event.button !== 0 ||
              event.metaKey ||
              event.ctrlKey ||
              event.shiftKey ||
              event.altKey
            ) {
              return;
            }
            event.preventDefault();
            onNavigate(target);
          }}
        >
          {children}
        </a>
      );
    };
    return createManualMarkdownComponents(headingIds, renderLink);
  }, [markdown, location.page, onNavigate]);

  const className = [
    "help-panel",
    fullWidth ? "is-full" : "",
    aboveDialogs ? "is-above-dialogs" : "",
  ]
    .filter(Boolean)
    .join(" ");
  const toggleLabel = fullWidth ? "Back to side panel" : "Open full width";

  return (
    <aside
      ref={panelRef}
      className={className}
      aria-label="Help"
      tabIndex={-1}
      style={{ "--help-top": `${top}px` } as CSSProperties}
    >
      <div className="help-panel-head">
        <div className="help-panel-heading">
          <span className="help-eyebrow">Help</span>
          <span className="help-crumb">{searching ? "Search" : title}</span>
        </div>
        <div className="help-panel-tools">
          <button
            type="button"
            className="icon-button"
            onClick={onBack}
            disabled={!canGoBack}
            aria-label="Back"
            title="Back"
          >
            <ArrowBackIcon />
          </button>
          <button
            type="button"
            className="icon-button"
            onClick={onHome}
            aria-label="Manual home"
            title="Manual home"
          >
            <HomeIcon />
          </button>
          <button
            type="button"
            className="icon-button help-full-toggle"
            onClick={onToggleFullWidth}
            aria-pressed={fullWidth}
            aria-label={toggleLabel}
            title={toggleLabel}
          >
            {fullWidth ? <CloseFullscreenIcon /> : <OpenInFullIcon />}
          </button>
          <button
            type="button"
            className="icon-button help-close"
            onClick={onClose}
            aria-label="Close Help"
            title="Close (Esc)"
          >
            <CloseIcon />
            <span className="help-close-text" aria-hidden="true">
              Close
            </span>
          </button>
        </div>
      </div>
      <form
        className="help-search"
        role="search"
        onSubmit={(event) => event.preventDefault()}
      >
        <input
          className="help-search-input"
          type="search"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          placeholder="Search the manual"
          aria-label="Search the manual"
        />
      </form>
      <div className="help-panel-body" ref={bodyRef}>
        <div className="help-panel-column">
          {searching ? (
            <HelpSearchResults
              search={search}
              onOpen={(page) => onNavigate({ page })}
              onBrowse={onHome}
            />
          ) : (
            <>
              {onOpenSetupChecklist && location.page === HELP_HOME ? (
                <HelpSetupChecklist onOpen={onOpenSetupChecklist} />
              ) : null}
              <HelpPage
                state={pageState}
                showStart={demoMode && location.page === HELP_HOME}
                components={components}
                onOpen={(page) => onNavigate({ page })}
                onHome={onHome}
              />
            </>
          )}
        </div>
      </div>
    </aside>
  );
}

function HelpPage({
  state,
  showStart,
  components,
  onOpen,
  onHome,
}: {
  state: PageState;
  showStart: boolean;
  components: ReturnType<typeof createManualMarkdownComponents>;
  onOpen: (page: string) => void;
  onHome: () => void;
}) {
  if (state.kind === "loading") {
    return <p className="help-status">Loading…</p>;
  }
  if (state.kind !== "page") {
    const message =
      state.kind === "missing"
        ? "This page is not in the manual for this version of Hatchdoor."
        : state.kind === "private"
          ? "This page is only shown after signing in."
          : "The manual could not be loaded. Check the connection and try again.";
    return (
      <div className="state-block help-state">
        <h2>Page not available</h2>
        <p>{message}</p>
        <button type="button" className="help-link" onClick={onHome}>
          Go to the manual's home page
        </button>
      </div>
    );
  }
  return (
    <>
      {showStart ? <HelpStart onOpen={onOpen} /> : null}
      <div className="note-body help-note-body" dir="auto">
        <ReactMarkdown
          remarkPlugins={REMARK_PLUGINS}
          rehypePlugins={REHYPE_PLUGINS}
          components={components}
        >
          {state.markdown}
        </ReactMarkdown>
      </div>
    </>
  );
}

/** Demo mode's Home view: installing comes first (#416, section B). */
function HelpStart({ onOpen }: { onOpen: (page: string) => void }) {
  return (
    <nav className="help-start" aria-label="Start here">
      <p className="help-start-label">Start here</p>
      <button
        type="button"
        className="help-start-card"
        onClick={() => onOpen(HELP_PAGES.install)}
      >
        <span className="help-start-title">Install Hatchdoor</span>
        <span className="help-start-blurb">
          Run your own copy with Docker Compose.
        </span>
      </button>
      <button
        type="button"
        className="help-start-card"
        onClick={() => onOpen(HELP_PAGES.deploy)}
      >
        <span className="help-start-title">Let your agent install it</span>
        <span className="help-start-blurb">
          Give your coding agent one line and answer its questions.
        </span>
      </button>
    </nav>
  );
}

/** Home's way back to the first-run checklist (#419) after it was closed. */
function HelpSetupChecklist({ onOpen }: { onOpen: () => void }) {
  return (
    <nav className="help-start" aria-label="Setup checklist">
      <button type="button" className="help-start-card" onClick={onOpen}>
        <span className="help-start-title">Setup checklist</span>
        <span className="help-start-blurb">
          Add your notes, connect your agent and try a search, one step at a
          time.
        </span>
      </button>
    </nav>
  );
}

function HelpSearchResults({
  search,
  onOpen,
  onBrowse,
}: {
  search: SearchState;
  onOpen: (page: string) => void;
  onBrowse: () => void;
}) {
  if (search.kind === "idle" || search.kind === "loading") {
    return <p className="help-status">Searching…</p>;
  }
  if (search.kind === "error") {
    return (
      <div className="state-block help-state">
        <h2>Search failed</h2>
        <p>
          The manual could not be searched. Check the connection and try again.
        </p>
      </div>
    );
  }
  if (search.hits.length === 0) {
    return (
      <div className="state-block help-state">
        <h2>Nothing matches “{search.query}”</h2>
        <p>
          Help searches the words in the manual. Try a shorter word, or one that
          names the screen you are on, such as “Git”, “token” or “Vault”.
        </p>
        <button type="button" className="help-link" onClick={onBrowse}>
          Browse every page instead
        </button>
      </div>
    );
  }
  return (
    <>
      <p className="help-search-count" role="status">
        {search.hits.length === 1
          ? `1 page matches “${search.query}”`
          : `${search.hits.length} pages match “${search.query}”`}
      </p>
      <ul className="search-results help-search-results">
        {search.hits.map((hit) => (
          <li key={hit.name} className="search-group">
            <button
              type="button"
              className="search-result search-result--primary"
              onClick={() => onOpen(hit.name)}
            >
              <div className="result-title">{hit.title}</div>
              <div className="result-path">
                <span className="result-path-text">{sectionOf(hit.name)}</span>
              </div>
              <p className="result-snippet">{plainExcerpt(hit.excerpt)}</p>
            </button>
          </li>
        ))}
      </ul>
    </>
  );
}

/** "guides/how-to-x" reads "Guides"; a top-level page has no section. */
function sectionOf(name: string): string {
  const slash = name.indexOf("/");
  if (slash < 0) {
    return "Manual";
  }
  const section = name.slice(0, slash).replace(/-/g, " ");
  return section.charAt(0).toUpperCase() + section.slice(1);
}
