export type FrontmatterEntryKind = "text" | "list";

export type FrontmatterEntry = {
  id: string;
  key: string;
  value: string;
  kind: FrontmatterEntryKind;
};

export function splitFrontmatter(content: string): {
  raw: string | null;
  body: string;
} {
  const lines = content.split(/\r?\n/);
  if (lines.length < 2 || lines[0].trim() !== "---") {
    return { raw: null, body: content };
  }

  const end = lines.findIndex(
    (line, index) => index > 0 && line.trim() === "---",
  );
  if (end === -1) {
    return { raw: null, body: content };
  }

  return {
    raw: lines.slice(1, end).join("\n"),
    body: lines.slice(end + 1).join("\n"),
  };
}

const PROPERTY_LINE = /^([A-Za-z0-9_][\w .-]*?)\s*:\s*(.*)$/;
const LIST_ITEM_LINE = /^\s{2,}-\s+(.*)$/;

export function parseFrontmatterEntries(raw: string): {
  editable: boolean;
  entries: FrontmatterEntry[];
} {
  const entries: FrontmatterEntry[] = [];
  let currentList: FrontmatterEntry | null = null;

  for (const line of raw.split(/\r?\n/)) {
    if (line.trim() === "") {
      continue;
    }

    const listItem = line.match(LIST_ITEM_LINE);
    if (listItem) {
      if (!currentList) {
        return { editable: false, entries: [] };
      }
      currentList.value = currentList.value
        ? `${currentList.value}, ${unquote(listItem[1])}`
        : unquote(listItem[1]);
      continue;
    }

    if (/^\s+\S/.test(line)) {
      return { editable: false, entries: [] };
    }

    const property = line.match(PROPERTY_LINE);
    if (!property) {
      return { editable: false, entries: [] };
    }

    const key = property[1].trim();
    const rawValue = property[2].trim();
    if (/^[|>&*]/.test(rawValue)) {
      return { editable: false, entries: [] };
    }

    const entry: FrontmatterEntry =
      rawValue === ""
        ? { id: key, key, value: "", kind: "list" }
        : { id: key, key, value: unquote(rawValue), kind: "text" };
    entries.push(entry);
    currentList = entry.kind === "list" ? entry : null;
  }

  return { editable: true, entries };
}

export function buildContentWithFrontmatter(
  entries: FrontmatterEntry[],
  body: string,
): string {
  const lines: string[] = [];

  for (const entry of entries) {
    const key = entry.key.trim();
    if (!key) {
      continue;
    }

    if (entry.kind === "list") {
      lines.push(`${key}:`);
      for (const item of entry.value.split(",").map((value) => value.trim())) {
        if (item) {
          lines.push(`  - ${serializeScalar(item)}`);
        }
      }
    } else {
      lines.push(`${key}: ${serializeScalar(entry.value.trim())}`);
    }
  }

  if (lines.length === 0) {
    return body;
  }
  return `---\n${lines.join("\n")}\n---\n${body}`;
}

function unquote(value: string): string {
  const trimmed = value.trim();
  if (
    trimmed.length >= 2 &&
    ((trimmed.startsWith('"') && trimmed.endsWith('"')) ||
      (trimmed.startsWith("'") && trimmed.endsWith("'")))
  ) {
    return trimmed.slice(1, -1);
  }
  return trimmed;
}

function serializeScalar(value: string): string {
  return needsQuote(value) ? JSON.stringify(value) : value;
}

function needsQuote(value: string): boolean {
  return (
    value === "" || /^[\s'"#-]/.test(value) || /[:#]\s|[:]$|\s$/.test(value)
  );
}
