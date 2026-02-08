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
