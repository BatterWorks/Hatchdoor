import { useMemo } from "react";

import ReactMarkdown from "react-markdown";
import rehypeKatex from "rehype-katex";
import remarkGfm from "remark-gfm";
import remarkMath from "remark-math";

import { parseFrontmatter, stripBlockIds } from "../../lib/markdown";
import { extractMarkdownHeadings } from "../../lib/noteHeadings";
import { NoteProperties } from "./sections";
import { createNoteMarkdownComponents } from "./renderers";
import { useResolvedWikilinks } from "./wikilinks";
import type { VaultId } from "../../types";

const PREVIEW_REHYPE_PLUGINS = [rehypeKatex];
const PREVIEW_REMARK_PLUGINS = [remarkGfm, remarkMath];

/**
 * Render draft markdown exactly as the read view would, so the editor preview
 * matches the published result (frontmatter as properties, resolved wikilinks,
 * KaTeX, GFM). Search highlighting is intentionally omitted — it has no meaning
 * while editing.
 */
export function NotePreview({
  vaultId,
  vaultName,
  content,
  relativePath,
}: {
  vaultId: VaultId;
  vaultName?: string;
  content: string;
  relativePath: string;
}) {
  const parsed = useMemo(() => parseFrontmatter(content), [content]);
  const { resolved: markdown } = useResolvedWikilinks(
    vaultId,
    stripBlockIds(parsed.body),
    relativePath,
  );
  const headingIdsBySourceLine = useMemo(
    () =>
      new Map(
        extractMarkdownHeadings(parsed.body).map(({ sourceLine, id }) => [
          sourceLine,
          id,
        ]),
      ),
    [parsed.body],
  );
  const components = useMemo(
    () =>
      createNoteMarkdownComponents(
        vaultId,
        relativePath,
        headingIdsBySourceLine,
      ),
    [vaultId, relativePath, headingIdsBySourceLine],
  );

  return (
    <div className="note-editor-preview note-body">
      <NoteProperties
        properties={parsed.properties}
        vaultName={vaultName}
        collapsed={false}
        onToggleCollapsed={() => {}}
        onTagSelect={() => {}}
      />
      <ReactMarkdown
        remarkPlugins={PREVIEW_REMARK_PLUGINS}
        rehypePlugins={PREVIEW_REHYPE_PLUGINS}
        components={components}
      >
        {markdown}
      </ReactMarkdown>
    </div>
  );
}
