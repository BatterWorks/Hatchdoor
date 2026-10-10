---
tags: [type/reference, topic/markdown]
---

# Supported Markdown reference

The Markdown features Hatchdoor renders. Every note in a Vault is a plain `.md` file, and this page lists what that file can contain and how Hatchdoor displays it. To see every feature rendered on one page, open [[Markdown feature showcase]].

## Inline formatting

Plain text can include **bold**, *italic*, ***bold italic***, ~~strikethrough~~, `inline code`, and links such as <https://example.com>.

Raw HTML is not supported. Use Markdown syntax so a note reads the same in other apps.

## Headings

`#` through `######` produce heading levels 1 through 6. Each heading gets an ID, which a heading wikilink and the table of contents use.

Those IDs, and the address a note itself answers to in the URL, are built the same way. An accent folds into the letter it sits on, so `## Café` is addressed as `cafe` and `## Straße` as `strasse`, while a heading in Cyrillic, Greek, Devanagari or a CJK script keeps its own letters rather than being spelled out in ASCII. Punctuation and symbols are dropped. Renaming a note to add or remove an accent leaves its address alone. A note whose name already carried one moved address when this rule landed, so a link to that note saved outside Hatchdoor may need updating.

## Lists

Unordered lists (`-`), ordered lists (`1.`), and nested lists at any depth are all supported. Task lists use `- [x]` (done) and `- [ ]` (open).

## Tables

```markdown
| Feature | Markdown trigger |
| --- | --- |
| Wikilink | `[[Note]]` |
| Callout | `> [!note]` |
```

A table wider than the note's column scrolls sideways inside its frame, on any screen.

## Blockquotes and callouts

A plain blockquote (`>`) is an ordinary quoted excerpt. A callout is a blockquote whose first line is `[!type]`, optionally followed by a custom title:

```markdown
> [!note]
> A note callout for neutral information.

> [!warning] Read this first
> A warning callout with a custom title.
```

Supported types: `note`, `info`, `tip`, `warning`, `danger`, `success`, `question`, `example`, `summary` and `abstract`, plus the other names Obsidian accepts, such as `tldr`, `hint`, `important`, `caution`, `bug`, `failure` and `faq`. The type sets the icon and the colour and nothing else. Add `+` after the type to make it collapsible and start open, or `-` to start closed: `> [!summary]+`.

## Code blocks

Fenced code blocks (` ``` `) render with syntax highlighting when a language is given (` ```rust `, ` ```bash `, ` ```json `, and so on) and as a plain block when it's omitted.

## Mermaid diagrams

A fenced block with the `mermaid` language renders as a diagram instead of code:

````markdown
```mermaid
flowchart LR
    A[Markdown files] --> B[Hatchdoor index]
    B --> C[Search]
```
````

## Saved queries

A fenced block with the `base` language is a saved query: a description of which notes to list, written in part of the syntax Obsidian's Bases feature uses. The note page draws it as a table of the notes it selects, one row per note, each linking to that note:

````markdown
<!-- hatchdoor-query: active-subscriptions -->
```base
filters:
  and:
    - file.hasTag("type/entity/subscription")
    - 'finished == null || finished > now()'
views:
  - type: table
    name: Active subscriptions
    order:
      - file.name
      - price
      - billing_period
      - finished
```
````

The block below is a live one rather than an example. It lists every reference note in this documentation, so in Hatchdoor you see the table it draws, and anywhere else you see the definition:

<!-- hatchdoor-query: reference-notes -->
```base
filters:
  and:
    - file.hasTag("type/reference")
views:
  - type: table
    name: Reference notes
    order:
      - file.name
      - file.tags
```

The table is worked out each time the note is read, from the Vault the note lives in and no other. It is never written into the file, so search, backlinks, the graph and statistics see the block's text and not the rows, and another Markdown app shows the block itself. A filter comparing against `now()` therefore changes its answer as time passes, with no edit to any file. Rows come back sorted by note title; the definition cannot change that order.

What a saved query can contain:

- `filters`, at the top and on the view, each either one expression in quotes or an `and`, `or` or `not` list of them. `not` means none of the listed conditions holds. The two sets of filters must both hold.
- Expressions that compare a property with a value: `price > 10`, `status != "done"`, `finished == null`. A property is named bare (`price`), as `note.price`, or as `note["next-payment"]` when its name has a hyphen. The file itself is `file.name` (with `.md`), `file.basename` (without), `file.path` and `file.folder`. Values are quoted text, numbers, `true`, `false`, `null`, `now()` and `today()`. Comparisons are `==`, `!=`, `<`, `<=`, `>` and `>=`, combined with `&&`, `||`, `!` and parentheses. Dates are compared as written, so write them as `2026-09-18`.
- `file.hasTag("a", "b")` for notes carrying any of those tags or one nested under them, `file.inFolder("projects")` for notes in that folder or below it, and `price.isEmpty()` for a property that is missing or blank.
- One view, of `type: table`, with an optional `name` shown above the table, a `limit` on how many rows it shows, and `order`, the columns from left to right. Without `order` the table has one column, the file name. `file.tags` is also available as a column, listing the note's frontmatter and inline tags together, and so is `aliases` (or `note.aliases`). A `tags` or `note.tags` column is refused, because Hatchdoor keeps only the combined list and would show more than the frontmatter property holds; use `file.tags` instead.

A property a note does not have counts as `null`: `finished == null` selects it, `status != "done"` selects it, and `price > 10` does not.

Anything else that could change which notes appear is refused rather than half applied. That includes formulas, `properties`, more than one view, `sort`, and any function not listed above. A refused block shows **Not evaluated** in place of its table, followed by a sentence naming the part Hatchdoor could not use, such as `daysUntil()` or a line of broken YAML.

Three things only change how the rows would be drawn, so Hatchdoor draws the table anyway, with every row it would otherwise show, and says under it what it left out: `groupBy` (the rows appear ungrouped), `summaries` (no summary row), and a view `type` other than `table`, such as `cards` or `list` (drawn as a table).

A block that is fine but matches no note shows **No matches.** and says it checked every note in the Vault, so it cannot be confused with a broken one. Click a column heading to sort that table by it: once for ascending, again for descending, a third time for the original order. The sort is only on your screen. It is not written into the note and a reload forgets it, the way Obsidian treats sorting. See [[How to troubleshoot common problems]] for the other messages a saved query can show.

An HTML comment of the form `<!-- hatchdoor-query: name -->` on the line before the block, with nothing but blank lines between them, gives the saved query a name. The name is lowercase letters, digits and hyphens and must be unique within the note. It is never shown on the page. A name that is not usable, or that two blocks share, gets a note under the table and the rows still appear, since a name never changes which rows qualify. A marker with no block after it gets a note where it sits. The marker sits outside the block so that Obsidian reads the block unchanged.

An agent connected over MCP reads the same rows as data. `get_note` lists a note's saved queries by name, and `evaluate_saved_query` returns one query's rows. The name is how an agent picks a saved query when a note holds more than one. See [[MCP tools reference#Reading a note's saved queries]].

## Math

Inline math uses single `$...$`; block math uses `$$...$$` on its own lines. Both render with KaTeX.

## Images and PDFs

Local Markdown image syntax works as expected:

```markdown
![Alt text](image-file-name.jpg)
```

Store an image near the note that references it, and give it a safe filename: lowercase ASCII letters, numbers and hyphens. The path is read the way a Markdown note link's is: a leading `/` starts at the Vault root, a bare filename is found anywhere in the Vault, and a space can be written as `%20`. A Markdown link to a local PDF is marked as a document and opens in a new tab: `[Open the report](report.pdf)`. The same attachment can instead be embedded inline, with page controls, using Obsidian's embed syntax: `![[report.pdf]]`.

## Wikilinks

Hatchdoor resolves `[[Note Title]]` to another note in the same Vault, and refreshes those links whenever Markdown changes. Five forms:

- Plain: `[[Connect your agent]]` → [[Connect your agent]]
- Aliased: `[[Connect your agent|connect an agent]]` shows the text after the `|`.
- Aliased inside a table cell: `[[Connect your agent\|connect an agent]]`. A bare `|` would end the cell, so the alias pipe is written escaped. Hatchdoor reads that backslash as syntax rather than as part of the note's name, so the link resolves, shows up in backlinks, and is rewritten by a rename like any other.
- Heading-scoped: `[[Connect your agent#Configure your MCP client]]` opens the note at that heading.
- To a note that does not exist yet: `[[This Note Does Not Exist Yet]]` still renders, and leads nowhere until that note is created.

## Markdown links to notes

A plain Markdown link whose target is a `.md` file is a note link too, the form Obsidian writes when its **Use `[[Wikilinks]]`** setting is off and most other Markdown tools write by default. It opens the note inside Hatchdoor and counts toward backlinks, the **Links** section under the note, the graph and statistics as a wikilink does. A wikilink and a Markdown link from one note to the same note count as one link.

- Relative to the note: `[the launch plan](../20-projects/Beacon%20Launch.md)`
- From the Vault root: `[the launch plan](/20-projects/Beacon%20Launch.md)`
- With the path in angle brackets, so spaces need no escaping: `[the launch plan](<../20-projects/Beacon Launch.md>)`
- Reference style: `[the launch plan][launch]`, with `[launch]: ../20-projects/Beacon%20Launch.md` on a line of its own
- Straight to a heading: `[first run](Install.md#First%20Run)` or `[first run](Install.md#first-run)`. Both reach the same heading, and a heading that no longer exists opens the note at the top.

The target is a path, not a title. Hatchdoor reads it the way it reads an attachment path: a leading `/` starts at the Vault root, otherwise it is tried from the linking note's own folder and then from the Vault root, and a bare filename such as `Beacon%20Launch.md` is found anywhere in the Vault, the nearest note of that name winning. A path that names a folder never falls back to a note of the same name elsewhere, so with two notes called `Plan`, `../a/Plan.md` always reaches `a/Plan`. Write a space as `%20`; a `%` that is not followed by two hex digits is read as a literal percent sign, so `Save%2020%%20now.md` still reaches `Save 20% now.md`. A target that names no note renders as a missing link rather than a link to nowhere.

Only a target ending in `.md` counts. A link written without the extension, a link to any other file or to a web address, and image syntax pointing at a `.md` file, `![x](Note.md)`, are left as they always were. Links inside code are not links.

When a note is renamed or moved, Hatchdoor rewrites the Markdown links that point at it and keeps the form you wrote: a relative path is worked out again from the linking note, a path starting with `/` keeps its `/`, and a bare filename stays bare while that name still reaches the note. The link text is never changed. A reference-style link changes on its definition line only. Moving the note that holds the links rewrites them from its new folder so they still reach the same notes. Deleting a note removes each Markdown link to it and keeps the link text as plain words, so `see [the plan](Plan.md) today` becomes `see the plan today`. When Hatchdoor writes a path it escapes only spaces, `%`, `#`, square brackets and parentheses; accented and non-Latin letters stay readable.

## Which link style Hatchdoor writes

A Vault has one link style, wikilinks or Markdown links, and every link Hatchdoor inserts follows it: a note picked from the editor's `[[` suggestions, and an attachment pasted, uploaded or dropped into a note. You don't set it in Hatchdoor. It is read from the Vault each time:

- If the Vault has Obsidian's settings file, `.obsidian/app.json`, its **Use `[[Wikilinks]]`** switch decides. Obsidian stores the switch turned off as `"useMarkdownLinks": true`. With the switch on, the key missing, or a file Hatchdoor cannot read, the Vault uses wikilinks, as Obsidian does.
- Otherwise Hatchdoor counts the note links and attachment embeds already in the Vault, and the form most of them use wins. No links at all, or an exact tie, means wikilinks.

In a wikilink Vault, Hatchdoor inserts `[[Note Title]]` and `![[path/to/file.png]]`. In a Markdown-link Vault it inserts `[Note Title](path/to/Note%20Title.md)` and `![](path/to/file.png)`. The link text is the note's title, with any `[`, `]`, `` ` `` or `\` escaped.

The path in a Markdown link has one of three forms. With Obsidian's settings file, its **New link format** setting picks it (`newLinkFormat` in the file, **Shortest path when possible** when the key is missing). Without one, Hatchdoor writes relative paths.

| Form | Example from `Projects/Plan.md` | Written as |
|---|---|---|
| Relative | `../Areas/Budget.md` | From the linking note's folder |
| Absolute | `/Areas/Budget.md` | From the Vault root, with a leading `/` |
| Shortest | `Budget.md` | The bare file name when no other file in the Vault shares it, otherwise the path from the Vault root, or with a leading `/` when that path would be read from the note's folder first |

Paths are escaped the way a rename rewrites them. Change the switch in Obsidian and the next link Hatchdoor inserts follows it, with no restart; the editor re-reads the Vault's style when it opens and before each attachment upload. Hatchdoor reads `.obsidian/app.json` and never writes to it. Agents see the same two values on the Vault, as `link_style` and `link_path_form`, and are expected to match them; Hatchdoor never rewrites the text an agent sends.

## Horizontal rule

Three hyphens (`---`) on a line of their own draw a horizontal rule, which divides a long note into sections. At the very top of a file the same three hyphens open the frontmatter block instead.

## Frontmatter

A note may open with a YAML frontmatter block:

```yaml
---
tags: [type/reference]
status: current
---
```

Hatchdoor reads the block and shows its properties, such as tags, aliases and any other key with its value, above the note body and apart from its text.

## Tags

A note's tags come from two places. Anything listed under `tags:` in frontmatter counts, in whatever shape you write it, including plain words and numbers.

In the body, only a namespaced hashtag counts: `#area/health` and `#type/reference/draft` are tags, a bare `#todo` or `#1177` is not. The namespace requirement is what keeps ordinary prose, issue numbers, and headings out of your tag list.

Hashtags inside code are never tags, so a note that documents a tag convention does not get filed under it. That covers both a fenced block and an inline span written with backticks:

````markdown
```
#not/a-tag
```

Write it as `#not/a-tag` in your note.
````

An agent renaming a tag across the Vault with `rename_tag` goes by these same rules, so a hashtag in code is left as written. `delete_tag` does too: a hashtag in code is not an inline use of the tag, so it does not block a delete.

---

Related: [[Markdown feature showcase]] · [[MCP tools reference]] · [[HTTP API reference]]
