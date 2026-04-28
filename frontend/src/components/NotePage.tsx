import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";

import ReactMarkdown from "react-markdown";
import { useLocation, useParams } from "react-router-dom";
import rehypeKatex from "rehype-katex";
import remarkGfm from "remark-gfm";
import remarkMath from "remark-math";

import { parseFrontmatter } from "../markdown";
import { extractMarkdownHeadings } from "../noteHeadings";
import {
  createSearchHighlightPlugin,
  normalizeSearchQuery,
  setActiveSearchHit as setActiveSearchHitClass,
} from "../noteSearch";
import { isNoteEqual, isNoteLinksEqual } from "../stateCompare";
import type {
  ActiveNoteMeta,
  Note,
  NoteLinks,
  NoteLinksResponse,
} from "../types";
import { NoteSkeleton, StateBlock, StatusBadge } from "./ui";
import { scrollElementIntoView } from "./note-page/dom";
import { createNoteMarkdownComponents } from "./note-page/renderers";
import {
  NoteLinksPanel,
  NoteProperties,
  NoteTocDesktop,
  NoteTocMobile,
  SearchHitNavigator,
} from "./note-page/sections";
import { useResolvedWikilinks } from "./note-page/wikilinks";

export function NotePage({
  onActiveNoteChange,
  onTagSelect,
  propertiesCollapsedStorageKey,
}: {
  onActiveNoteChange: (meta: ActiveNoteMeta | null) => void;
  onTagSelect: (tag: string) => void;
  propertiesCollapsedStorageKey: string;
}) {
  const params = useParams<{ slug: string }>();
  const location = useLocation();
  const slug = params.slug ?? "";
  const [note, setNote] = useState<Note | null>(null);
  const [noteLinks, setNoteLinks] = useState<NoteLinks | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [propertiesCollapsed, setPropertiesCollapsed] = useState<boolean>(
    () => {
      return window.localStorage.getItem(propertiesCollapsedStorageKey) !== "0";
    },
  );
  const [searchHitCount, setSearchHitCount] = useState(0);
  const [activeSearchHit, setActiveSearchHit] = useState(0);
  const noteBodyRef = useRef<HTMLDivElement | null>(null);
  const searchHitsRef = useRef<HTMLSpanElement[]>([]);

  const loadNote = useCallback(
    async (hardReload: boolean) => {
      setError(null);
      if (hardReload) {
        setNote(null);
      }

      try {
        const res = await fetch(`/api/note/${encodeURIComponent(slug)}`);
        if (!res.ok) {
          throw new Error(`Failed loading note: ${res.status}`);
        }
        const json = (await res.json()) as { note: Note };
        setNote((prev) => (isNoteEqual(prev, json.note) ? prev : json.note));
      } catch (err) {
        setError(
          err instanceof Error ? err.message : "Unknown note loading error",
        );
      }
    },
    [slug],
  );

  const loadNoteLinks = useCallback(async () => {
    try {
      const res = await fetch(`/api/note/${encodeURIComponent(slug)}/links`);
      if (!res.ok) {
        throw new Error(`Failed loading note links: ${res.status}`);
      }
      const json = (await res.json()) as NoteLinksResponse;
      setNoteLinks((prev) =>
        isNoteLinksEqual(prev, json.links) ? prev : json.links,
      );
    } catch {
      setNoteLinks(null);
    }
  }, [slug]);

  useEffect(() => {
    void (async () => {
      setLoading(true);
      await loadNote(true);
      await loadNoteLinks();
      setLoading(false);
    })();
  }, [loadNote, loadNoteLinks]);

  useEffect(() => {
    const id = window.setInterval(() => {
      void loadNote(false);
      void loadNoteLinks();
    }, 10_000);

    return () => {
      window.clearInterval(id);
    };
  }, [loadNote, loadNoteLinks]);

  useEffect(() => {
    if (!note) {
      onActiveNoteChange(null);
      return;
    }

    onActiveNoteChange({
      title: note.title,
      slug: note.slug,
      relativePath: note.relative_path,
    });
  }, [note, onActiveNoteChange]);

  useEffect(() => {
    window.localStorage.setItem(
      propertiesCollapsedStorageKey,
      propertiesCollapsed ? "1" : "0",
    );
  }, [propertiesCollapsed, propertiesCollapsedStorageKey]);

  useEffect(() => {
    const onToggle = () => setPropertiesCollapsed((prev) => !prev);
    window.addEventListener("hatchdoor:toggle-note-properties", onToggle);
    return () =>
      window.removeEventListener("hatchdoor:toggle-note-properties", onToggle);
  }, []);

  const parsed = useMemo(() => parseFrontmatter(note?.content ?? ""), [note]);
  const markdown = useResolvedWikilinks(parsed.body, note?.relative_path ?? "");
  const searchQuery = useMemo(
    () => normalizeSearchQuery(new URLSearchParams(location.search).get("q")),
    [location.search],
  );
  const tocHeadings = useMemo(
    () => extractMarkdownHeadings(parsed.body),
    [parsed.body],
  );
  const rehypePlugins = useMemo(
    () => [rehypeKatex, createSearchHighlightPlugin(searchQuery)],
    [searchQuery],
  );

  useLayoutEffect(() => {
    const root = noteBodyRef.current;
    if (!root) {
      return;
    }

    const hits = Array.from(
      root.querySelectorAll<HTMLSpanElement>("mark.search-hit"),
    );
    searchHitsRef.current = hits;
    setSearchHitCount(hits.length);
    setActiveSearchHit(0);

    if (hits.length > 0) {
      setActiveSearchHitClass(hits, 0);
      scrollElementIntoView(hits[0], { block: "center", inline: "nearest" });
    }

    return () => {
      searchHitsRef.current = [];
    };
  }, [markdown, note?.slug, searchQuery]);

  useEffect(() => {
    if (searchHitsRef.current.length === 0) {
      return;
    }
    setActiveSearchHitClass(searchHitsRef.current, activeSearchHit);
  }, [activeSearchHit]);

  if (loading) {
    return <NoteSkeleton />;
  }
  if (error && !note) {
    return (
      <StateBlock
        title="Note Unavailable"
        description={error}
        actionLabel="Retry"
        onAction={() => void loadNote(true)}
      />
    );
  }
  if (!note) {
    return (
      <StateBlock title="Not Found" description="This note no longer exists." />
    );
  }

  const headingCounts = new Map<string, number>();

  return (
    <div className="note-page-layout">
      <article className="note-content">
        <h2 className="note-page-title">{note.title}</h2>
        {error ? <StatusBadge tone="warn" text="Showing cached note" /> : null}
        {searchHitCount > 0 ? (
          <SearchHitNavigator
            totalHits={searchHitCount}
            activeHit={activeSearchHit}
            onSelect={(nextIndex) => {
              setActiveSearchHit(nextIndex);
              const target = searchHitsRef.current[nextIndex];
              scrollElementIntoView(target, {
                block: "center",
                inline: "nearest",
              });
            }}
          />
        ) : null}
        <NoteProperties
          properties={parsed.properties}
          collapsed={propertiesCollapsed}
          onToggleCollapsed={() => setPropertiesCollapsed((prev) => !prev)}
          onTagSelect={onTagSelect}
        />
        <NoteLinksPanel links={noteLinks} />
        <NoteTocMobile headings={tocHeadings} />
        <div ref={noteBodyRef} className="note-body">
          <ReactMarkdown
            remarkPlugins={[remarkGfm, remarkMath]}
            rehypePlugins={rehypePlugins}
            components={createNoteMarkdownComponents(
              note.relative_path,
              headingCounts,
            )}
          >
            {markdown}
          </ReactMarkdown>
        </div>
      </article>

      <NoteTocDesktop headings={tocHeadings} />
    </div>
  );
}
