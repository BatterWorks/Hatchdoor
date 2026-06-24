export type ConflictDiffLine =
  | { kind: "same"; text: string }
  | { kind: "disk"; text: string }
  | { kind: "draft"; text: string };

export function diffConflictLines(
  diskContent: string,
  draftContent: string,
): ConflictDiffLine[] {
  const diskLines = diskContent.split("\n");
  const draftLines = draftContent.split("\n");
  const max = Math.max(diskLines.length, draftLines.length);
  const lines: ConflictDiffLine[] = [];

  for (let index = 0; index < max; index += 1) {
    const disk = diskLines[index];
    const draft = draftLines[index];
    if (disk === draft) {
      lines.push({ kind: "same", text: disk ?? "" });
      continue;
    }
    if (disk !== undefined) {
      lines.push({ kind: "disk", text: disk });
    }
    if (draft !== undefined) {
      lines.push({ kind: "draft", text: draft });
    }
  }

  return lines;
}
