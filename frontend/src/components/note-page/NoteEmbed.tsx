import {
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import ReactMarkdown from "react-markdown";
import { Link } from "react-router-dom";
import rehypeKatex from "rehype-katex";
import type { PluggableList } from "unified";

import { apiFetch } from "../../api/api";
import { parseFrontmatter } from "../../lib/markdown";
import { NOTE_SYNTAX_PLUGINS } from "../../lib/markdownSyntax";
import { hideComments } from "../../lib/noteComments";
import {
  duplicateTitleHeadingLine,
  extractMarkdownHeadings,
} from "../../lib/noteHeadings";
import type { Note, VaultId, VaultQualifiedNote } from "../../types";
import {
  NoteEmbedContext,
  mayEmbed,
  rehypeDropIds,
  remarkSelectEmbed,
  selectEmbed,
  type NoteEmbedScope,
  type NoteEmbedTarget,
} from "./noteEmbeds";
import { createNoteMarkdownComponents } from "./renderers";
import { SavedQueryProvider } from "./SavedQueryBlock";
import { remarkHideQueryMarkers, useSavedQueries } from "./savedQueries";
import { useResolvedWikilinks } from "./wikilinks";

const EMBED_REHYPE_PLUGINS = [rehypeKatex, rehypeDropIds];
const NO_HEADING_IDS = new Map<number, string>();

/**
 * A note embed (#699) drawn as the content it names: the whole note, one
 * heading's section or one block, read through the same note read the page
 * uses, inside a frame that names and links its source. Anything that stops
 * it (no such note, heading or block, a failed read, an embed of a note it
 * is already inside, more than `MAX_EMBED_DEPTH` levels) draws the plain link
 * instead.
 */
export function NoteEmbed({
  vaultId,
  target,
  children,
}: {
  vaultId: VaultId;
  target: NoteEmbedTarget;
  /** The embed's label, which the frame and the fallback link show. */
  children?: ReactNode;
}) {
  const scope = useContext(NoteEmbedContext);
  const href = noteHref(vaultId, target);
  if (!mayEmbed(scope, vaultId, target.slug)) {
    return <EmbedLink href={href} target={target} label={children} />;
  }
  return (
    // Keyed by the note, so an embed retargeted to another note never shows
    // the previous one's content while the new read is on its way.
    <LoadedEmbed
      key={target.slug}
      scope={scope}
      target={target}
      href={href}
      label={children}
    />
  );
}

function noteHref(vaultId: VaultId, target: NoteEmbedTarget): string {
  const anchor = target.anchor ? `#${target.anchor}` : "";
  return `/v/${encodeURIComponent(vaultId)}/n/${target.slug}${anchor}`;
}

function EmbedLink({
  href,
  target,
  label,
}: {
  href: string;
  target: NoteEmbedTarget;
  label: ReactNode;
}) {
  return (
    <Link
      className={target.archived ? "archived-link" : undefined}
      to={href}
      data-note-embed-fallback=""
    >
      {label}
    </Link>
  );
}

type ReadState =
  { kind: "loading" } | { kind: "ready"; note: Note } | { kind: "failed" };

function LoadedEmbed({
  scope,
  target,
  href,
  label,
}: {
  scope: NoteEmbedScope;
  target: NoteEmbedTarget;
  href: string;
  label: ReactNode;
}) {
  const [read, setRead] = useState<ReadState>({ kind: "loading" });
  const notePath = `/api/v1/vaults/${encodeURIComponent(scope.vaultId)}/notes/${encodeURIComponent(target.slug)}`;

  // Read again when the collection moves on, so an edit to the embedded note
  // shows; the content on screen stays until the new read lands. A read that
  // hangs ends at `apiFetch`'s timeout and draws the link.
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const res = await apiFetch(notePath);
        if (!res.ok) {
          throw new Error(`note read ${res.status}`);
        }
        const json = (await res.json()) as Partial<VaultQualifiedNote>;
        // A hand-written `/__embed__/` link can name something that is not a
        // note; anything without a note's text draws the link instead.
        if (typeof json.note?.content !== "string") {
          throw new Error("not a note");
        }
        const note = json.note;
        if (!cancelled) {
          setRead((previous) =>
            previous.kind === "ready" &&
            previous.note.content_hash === note.content_hash
              ? previous
              : { kind: "ready", note },
          );
        }
      } catch {
        if (!cancelled) {
          setRead({ kind: "failed" });
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [notePath, scope.revision]);

  if (read.kind === "failed") {
    return <EmbedLink href={href} target={target} label={label} />;
  }
  if (read.kind === "loading") {
    return (
      <EmbedFrame href={href} label={label} busy>
        <p className="note-embed-status">Loading…</p>
      </EmbedFrame>
    );
  }
  return (
    <EmbeddedBody
      scope={scope}
      target={target}
      note={read.note}
      notePath={notePath}
      href={href}
      label={label}
    />
  );
}

function EmbeddedBody({
  scope,
  target,
  note,
  notePath,
  href,
  label,
}: {
  scope: NoteEmbedScope;
  target: NoteEmbedTarget;
  note: Note;
  notePath: string;
  href: string;
  label: ReactNode;
}) {
  const body = useMemo(
    () => hideComments(parseFrontmatter(note.content).body),
    [note.content],
  );
  const selection = useMemo(
    () => selectEmbed(body, target.anchor),
    [body, target.anchor],
  );
  const { resolved } = useResolvedWikilinks(
    scope.vaultId,
    body,
    note.relative_path,
  );
  const savedQueries = useSavedQueries(
    notePath,
    note.content,
    note.content_hash,
    scope.revision,
    selection !== null,
  );
  // A whole note that opens by repeating its title does not say it twice
  // under a frame that already names it (#530).
  const hiddenHeadingLine = useMemo(
    () =>
      selection?.kind === "whole"
        ? duplicateTitleHeadingLine(
            body,
            extractMarkdownHeadings(body),
            note.title,
          )
        : undefined,
    [selection, body, note.title],
  );
  const components = useMemo(
    () =>
      createNoteMarkdownComponents(
        scope.vaultId,
        note.relative_path,
        NO_HEADING_IDS,
        { hiddenHeadingLine, embedded: true },
      ),
    [scope.vaultId, note.relative_path, hiddenHeadingLine],
  );
  const remarkPlugins = useMemo<PluggableList>(
    () =>
      selection
        ? [
            ...NOTE_SYNTAX_PLUGINS,
            remarkHideQueryMarkers,
            [remarkSelectEmbed, { selection }],
          ]
        : [],
    [selection],
  );
  const inner = useMemo(
    () => ({ ...scope, chain: [...scope.chain, target.slug] }),
    [scope, target.slug],
  );

  if (!selection) {
    return <EmbedLink href={href} target={target} label={label} />;
  }

  return (
    <EmbedFrame href={href} label={label}>
      <NoteEmbedContext.Provider value={inner}>
        <SavedQueryProvider
          state={savedQueries}
          vaultId={scope.vaultId}
          markdown={resolved}
        >
          <ReactMarkdown
            remarkPlugins={remarkPlugins}
            rehypePlugins={EMBED_REHYPE_PLUGINS}
            components={components}
          >
            {resolved}
          </ReactMarkdown>
        </SavedQueryProvider>
      </NoteEmbedContext.Provider>
    </EmbedFrame>
  );
}

function EmbedFrame({
  href,
  label,
  busy = false,
  children,
}: {
  href: string;
  label: ReactNode;
  busy?: boolean;
  children: ReactNode;
}) {
  return (
    <div
      className="note-embed"
      data-testid="note-embed"
      role="group"
      aria-label="Embedded note"
      aria-busy={busy || undefined}
    >
      <div className="note-embed-head">
        <span className="note-embed-eyebrow">Embedded</span>
        <Link className="note-embed-source" to={href}>
          {label}
        </Link>
      </div>
      <div className="note-embed-body">{children}</div>
    </div>
  );
}
