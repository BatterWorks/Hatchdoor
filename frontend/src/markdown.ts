export function parseWikilinkTarget(body: string): {
  target: string;
  label: string;
} {
  const [targetRaw, aliasRaw] = body.split("|", 2);
  const target = (targetRaw || "").trim();
  const label = (aliasRaw || "").trim() || target;
  return { target, label };
}

export function escapeMarkdownLabel(input: string): string {
  return input.replace(/[\\`*_[\]{}()#+.!|]/g, "\\$&");
}

export type FrontmatterValue = string | string[];

export function parseFrontmatter(input: string): {
  properties: Record<string, FrontmatterValue>;
  body: string;
} {
  const lines = input.split(/\r?\n/);
  if (lines.length < 3 || lines[0].trim() !== "---") {
    return { properties: {}, body: input };
  }

  let end = -1;
  for (let idx = 1; idx < lines.length; idx += 1) {
    if (lines[idx].trim() === "---") {
      end = idx;
      break;
    }
  }

  if (end < 0) {
    return { properties: {}, body: input };
  }

  const header = lines.slice(1, end);
  if (!looksLikeFrontmatterHeader(header)) {
    return { properties: {}, body: input };
  }
  const body = lines.slice(end + 1).join("\n");
  const properties: Record<string, FrontmatterValue> = {};
  let listKey: string | null = null;

  for (const line of header) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith("#")) {
      continue;
    }

    const listMatch = line.match(/^\s*-\s+(.+)$/);
    if (listMatch && listKey) {
      const current = properties[listKey];
      const nextValue = parseScalar(listMatch[1]);
      if (Array.isArray(current)) {
        current.push(nextValue);
      } else {
        properties[listKey] = [nextValue];
      }
      continue;
    }

    const keyMatch = line.match(/^([^:\n][^:\n]*?)\s*:\s*(.*)$/);
    if (!keyMatch) {
      listKey = null;
      continue;
    }

    const key = normalizeKey(keyMatch[1]);
    if (!key) {
      listKey = null;
      continue;
    }
    const rawValue = keyMatch[2].trim();
    if (!rawValue) {
      properties[key] = [];
      listKey = key;
      continue;
    }

    properties[key] = parseValue(rawValue);
    listKey = null;
  }

  return { properties, body };
}

function parseValue(rawValue: string): FrontmatterValue {
  if (rawValue.startsWith("[") && rawValue.endsWith("]")) {
    const inner = rawValue.slice(1, -1).trim();
    if (!inner) {
      return [];
    }
    return inner.split(",").map((item) => parseScalar(item));
  }

  return parseScalar(rawValue);
}

function parseScalar(value: string): string {
  const trimmed = value.trim();
  if (
    (trimmed.startsWith('"') && trimmed.endsWith('"')) ||
    (trimmed.startsWith("'") && trimmed.endsWith("'"))
  ) {
    return trimmed.slice(1, -1).trim();
  }
  return trimmed;
}

function normalizeKey(rawKey: string): string {
  const trimmed = rawKey.trim();
  if (!trimmed) {
    return "";
  }
  if (
    (trimmed.startsWith('"') && trimmed.endsWith('"')) ||
    (trimmed.startsWith("'") && trimmed.endsWith("'"))
  ) {
    return trimmed.slice(1, -1).trim();
  }
  return trimmed;
}

function looksLikeFrontmatterHeader(lines: string[]): boolean {
  if (lines.length === 0) {
    return false;
  }

  let hasProperty = false;
  let listAllowed = false;

  for (const line of lines) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith("#")) {
      continue;
    }

    if (/^\s*-\s+/.test(line)) {
      if (!listAllowed) {
        return false;
      }
      hasProperty = true;
      continue;
    }

    if (/^[^:\n][^:\n]*\s*:/.test(trimmed)) {
      hasProperty = true;
      listAllowed = /:\s*$/.test(trimmed);
      continue;
    }

    return false;
  }

  return hasProperty;
}

export function normalizeTags(value: FrontmatterValue | undefined): string[] {
  if (value === undefined) {
    return [];
  }

  const parts = Array.isArray(value) ? value : [value];
  const tags = new Set<string>();

  for (const part of parts) {
    const segments = part.includes(",")
      ? part.split(",")
      : part.split(/\s+/).filter((item) => item.length > 0);

    for (const segment of segments) {
      const normalized = segment
        .trim()
        .replace(/^\[|\]$/g, "")
        .replace(/^#/, "");
      if (normalized) {
        tags.add(normalized);
      }
    }
  }

  return [...tags];
}
