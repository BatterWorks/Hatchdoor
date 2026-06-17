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
import { apiFetch } from "../api";
import rehypeKatex from "rehype-katex";
import remarkGfm from "remark-gfm";
import remarkMath from "remark-math";

import { parseFrontmatter, stripBlockIds, stripVaultNoteLinks } from "../markdown";
import { extractMarkdownHeadings, slugifyHeading } from "../noteHeadings";
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
import { updateNote } from "../writeApi";
import {
  clearNoteDraft,
  loadNoteDraft,
  saveNoteDraft,
} from "../writeDrafts";
import { NoteEditor } from "./NoteEditor";
import { NoteSkeleton, StateBlock, StatusBadge } from "./ui";
import { jumpToHeading, scrollElementIntoView } from "./note-page/dom";
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
  vaultRevision,
  writeEnabled,
  editRequestId,
}: {
  onActiveNoteChange: (meta: ActiveNoteMeta | null) => void;
  onTagSelect: (tag: string) => void;
  propertiesCollapsedStorageKey: string;
  vaultRevision: number;
  writeEnabled: boolean;
  editRequestId: number;
}) {
  const params = useParams<{ slug: string }>();
  const location = useLocation();
  const slug = params.slug ?? "";
  const [note, setNote] = useState<Note | null>(null);
  const [noteLinks, setNoteLinks] = useState<NoteLinks | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [isEditing, setIsEditing] = useState(false);
  const [draftContent, setDraftContent] = useState("");
  const [editorError, setEditorError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [propertiesCollapsed, setPropertiesCollapsed] = useState<boolean>(
    () => {
      return window.localStorage.getItem(propertiesCollapsedStorageKey) !== "0";
    },
  );
  const [searchHitCount, setSearchHitCount] = useState(0);
  const [activeSearchHit, setActiveSearchHit] = useState(0);
  const noteBodyRef = useRef<HTMLDivElement | null>(null);
  const searchHitsRef = useRef<HTMLSpanElement[]>([]);
  const currentSlugRef = useRef(slug);
  const lastEditRequestIdRef = useRef(editRequestId);
  currentSlugRef.current = slug;

  const loadNote = useCallback(
    async (hardReload: boolean) => {
      setError(null);
      if (hardReload) {
        setNote(null);
      }

      try {
        const res = await apiFetch(`/api/note/${encodeURIComponent(slug)}`);
        if (!res.ok) {
          throw new Error(`Failed loading note: ${res.status}`);
        }
        const json = (await res.json()) as { note: Note };
        if (slug !== currentSlugRef.current) return;
        setNote((prev) => (isNoteEqual(prev, json.note) ? prev : json.note));
      } catch (err) {
        if (slug !== currentSlugRef.current) return;
        setError(
          err instanceof Error ? err.message : "Unknown note loading error",
        );
      }
    },
    [slug],
  );

  const loadNoteLinks = useCallback(async () => {
    try {
      const res = await apiFetch(`/api/note/${encodeURIComponent(slug)}/links`);
      if (!res.ok) {
        throw new Error(`Failed loading note links: ${res.status}`);
      }
      const json = (await res.json()) as NoteLinksResponse;
      if (slug !== currentSlugRef.current) return;
      setNoteLinks((prev) =>
        isNoteLinksEqual(prev, json.links) ? prev : json.links,
      );
    } catch {
      if (slug !== currentSlugRef.current) return;
      setNoteLinks(null);
    }
  }, [slug]);

  useEffect(() => {
    let cancelled = false;

    void (async () => {
      setLoading(true);
      await loadNote(true);
      await loadNoteLinks();
      if (!cancelled) {
        setLoading(false);
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [loadNote, loadNoteLinks]);

  useEffect(() => {
    setIsEditing(false);
    setDraftContent("");
    setEditorError(null);
    setSaving(false);
  }, [slug]);

  useEffect(() => {
    if (vaultRevision === 0) {
      return;
    }

    void loadNote(false);
    void loadNoteLinks();
  }, [loadNote, loadNoteLinks, vaultRevision]);

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

  const startEditing = useCallback(() => {
    if (!writeEnabled || !note || isEditing) {
      return;
    }

    const storedDraft = loadNoteDraft(note.slug);
    setDraftContent(storedDraft?.content ?? note.content);
    setEditorError(null);
    setSaving(false);
    setIsEditing(true);
  }, [isEditing, note, writeEnabled]);

  useEffect(() => {
    if (editRequestId === lastEditRequestIdRef.current) {
      return;
    }

    lastEditRequestIdRef.current = editRequestId;
    startEditing();
  }, [editRequestId, startEditing]);

  useEffect(() => {
    if (!isEditing || !note) {
      return;
    }

    saveNoteDraft(note.slug, {
      slug: note.slug,
      content: draftContent,
      baseContentHash: note.content_hash,
      savedAt: Date.now(),
    });
  }, [draftContent, isEditing, note]);

  const parsed = useMemo(() => parseFrontmatter(note?.content ?? ""), [note]);

  useEffect(() => {
    if (!note) {
      onActiveNoteChange(null);
      return;
    }

    onActiveNoteChange({
      title: note.title,
      slug: note.slug,
      relativePath: note.relative_path,
      exportContent: stripVaultNoteLinks(parsed.body),
    });
  }, [note, onActiveNoteChange, parsed.body]);

  const markdown = useResolvedWikilinks(stripBlockIds(parsed.body), note?.relative_path ?? "");
  const searchQuery = useMemo(
    () => normalizeSearchQuery(new URLSearchParams(location.search).get("q")),
    [location.search],
  );
  const matchHeading = useMemo(
    () => new URLSearchParams(location.search).get("m"),
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
  const markdownComponents = useMemo(() => {
    const headingCounts = new Map<string, number>();
    return createNoteMarkdownComponents(note?.relative_path ?? "", headingCounts);
  }, [note?.relative_path]);

  useLayoutEffect(() => {
    const root = noteBodyRef.current;
    if (!root) {
      searchHitsRef.current = [];
      setSearchHitCount(0);
      setActiveSearchHit(0);
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
    } else if (matchHeading) {
      const lastSegment = matchHeading.split(" > ").at(-1) ?? matchHeading;
      jumpToHeading(slugifyHeading(lastSegment));
    }

    return () => {
      searchHitsRef.current = [];
    };
  }, [markdown, note?.slug, searchQuery, matchHeading]);

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

  const handleCancelEditing = () => {
    const isDirty = draftContent !== note.content;
    if (
      isDirty &&
      !window.confirm("Discard your unsaved draft for this note?")
    ) {
      return;
    }

    clearNoteDraft(note.slug);
    setDraftContent(note.content);
    setEditorError(null);
    setSaving(false);
    setIsEditing(false);
  };

  const handleSave = async () => {
    setSaving(true);
    setEditorError(null);

    try {
      await updateNote(note.slug, draftContent, note.content_hash);
      clearNoteDraft(note.slug);
      setIsEditing(false);
      setLoading(true);
      await loadNote(true);
      await loadNoteLinks();
    } catch (saveError) {
      if (saveError instanceof Error && saveError.name === "ConflictError") {
        setEditorError(
          "This note changed on disk. Your draft was kept; reload the latest note before saving.",
        );
      } else if (saveError instanceof Error) {
        setEditorError(saveError.message);
      } else {
        setEditorError("Failed saving note.");
      }
    } finally {
      setSaving(false);
      setLoading(false);
    }
  };

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
        {isEditing ? (
          <NoteEditor
            content={draftContent}
            saving={saving}
            error={editorError}
            onChange={setDraftContent}
            onSave={handleSave}
            onCancel={handleCancelEditing}
          />
        ) : (
          <div ref={noteBodyRef} className="note-body">
            <ReactMarkdown
              remarkPlugins={[remarkGfm, remarkMath]}
              rehypePlugins={rehypePlugins}
              components={markdownComponents}
            >
              {markdown}
            </ReactMarkdown>
          </div>
        )}
      </article>

      <NoteTocDesktop headings={tocHeadings} />
    </div>
  );
}
