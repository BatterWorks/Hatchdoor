export type ConflictDiffLine =
  | { kind: "same"; text: string }
  | { kind: "disk"; text: string }
  | { kind: "draft"; text: string }
  | { kind: "skip"; count: number };

/** Unchanged lines kept on each side of a change, so a hunk reads in place. */
const CONTEXT_LINES = 3;

/**
 * Past this many differing lines the two versions have little in common left
 * to align, and the search's cost grows with the square of it. The remaining
 * middle is shown as one replaced run instead.
 */
const MAX_EDIT_DISTANCE = 2000;

type Op = Exclude<ConflictDiffLine, { kind: "skip" }>;

/**
 * A line diff of the disk version against the draft (#331), with long runs of
 * unchanged lines folded into a single `skip` row.
 *
 * Lines are aligned by a shortest edit script (Myers), so a line inserted on
 * one side leaves everything after it matched rather than shifting every later
 * pair out of step, which is what comparing line `i` with line `i` did.
 */
export function diffConflictLines(
  diskContent: string,
  draftContent: string,
): ConflictDiffLine[] {
  return collapseUnchanged(
    diffLines(diskContent.split("\n"), draftContent.split("\n")),
  );
}

function diffLines(disk: string[], draft: string[]): Op[] {
  // The common head and tail never need searching, and are most of a note
  // that two people edited in one place each.
  let head = 0;
  while (
    head < disk.length &&
    head < draft.length &&
    disk[head] === draft[head]
  ) {
    head += 1;
  }
  let tail = 0;
  while (
    tail < disk.length - head &&
    tail < draft.length - head &&
    disk[disk.length - 1 - tail] === draft[draft.length - 1 - tail]
  ) {
    tail += 1;
  }

  const same = (text: string): Op => ({ kind: "same", text });
  return [
    ...disk.slice(0, head).map(same),
    ...diffMiddle(
      disk.slice(head, disk.length - tail),
      draft.slice(head, draft.length - tail),
    ),
    ...disk.slice(disk.length - tail).map(same),
  ];
}

/** Myers' O(ND) shortest edit script between two line arrays. */
function diffMiddle(a: string[], b: string[]): Op[] {
  const n = a.length;
  const m = b.length;
  if (n === 0 || m === 0) {
    return [
      ...a.map((text): Op => ({ kind: "disk", text })),
      ...b.map((text): Op => ({ kind: "draft", text })),
    ];
  }

  const max = Math.min(n + m, MAX_EDIT_DISTANCE);
  const offset = max + 1;
  const v = new Int32Array(2 * max + 3);
  // trace[d] is the frontier before step d, kept only over the diagonals step
  // d reads (-d..d), so memory grows with the edit distance, not the note.
  const trace: Int32Array[] = [];

  for (let d = 0; d <= max; d += 1) {
    trace.push(v.slice(offset - d, offset + d + 1));
    for (let k = -d; k <= d; k += 2) {
      let x =
        k === -d || (k !== d && v[offset + k - 1] < v[offset + k + 1])
          ? v[offset + k + 1]
          : v[offset + k - 1] + 1;
      let y = x - k;
      while (x < n && y < m && a[x] === b[y]) {
        x += 1;
        y += 1;
      }
      v[offset + k] = x;
      if (x >= n && y >= m) {
        return backtrack(a, b, trace);
      }
    }
  }

  // Too far apart to be worth aligning: the whole middle was replaced.
  return [
    ...a.map((text): Op => ({ kind: "disk", text })),
    ...b.map((text): Op => ({ kind: "draft", text })),
  ];
}

function backtrack(a: string[], b: string[], trace: Int32Array[]): Op[] {
  const ops: Op[] = [];
  let x = a.length;
  let y = b.length;
  for (let d = trace.length - 1; d >= 0; d -= 1) {
    const frontier = trace[d];
    const at = (k: number) => frontier[k + d];
    const k = x - y;
    let prevX = 0;
    let prevY = 0;
    if (d > 0) {
      const prevK =
        k === -d || (k !== d && at(k - 1) < at(k + 1)) ? k + 1 : k - 1;
      prevX = at(prevK);
      prevY = prevX - prevK;
    }
    while (x > prevX && y > prevY) {
      ops.push({ kind: "same", text: a[x - 1] });
      x -= 1;
      y -= 1;
    }
    if (d > 0) {
      if (x === prevX) {
        ops.push({ kind: "draft", text: b[y - 1] });
      } else {
        ops.push({ kind: "disk", text: a[x - 1] });
      }
    }
    x = prevX;
    y = prevY;
  }
  ops.reverse();
  return groupHunks(ops);
}

/** Within each changed run, every disk line before every draft line, so a
 * replaced paragraph reads as "was" then "now" rather than interleaved. */
function groupHunks(ops: Op[]): Op[] {
  const out: Op[] = [];
  let disk: Op[] = [];
  let draft: Op[] = [];
  const flush = () => {
    out.push(...disk, ...draft);
    disk = [];
    draft = [];
  };
  for (const op of ops) {
    if (op.kind === "same") {
      flush();
      out.push(op);
    } else if (op.kind === "disk") {
      disk.push(op);
    } else {
      draft.push(op);
    }
  }
  flush();
  return out;
}

/** Fold every unchanged run down to the context around the changes. A review
 * of a long note then shows the hunks, and stays a handful of rows. */
function collapseUnchanged(ops: Op[]): ConflictDiffLine[] {
  const out: ConflictDiffLine[] = [];
  let index = 0;
  while (index < ops.length) {
    if (ops[index].kind !== "same") {
      out.push(ops[index]);
      index += 1;
      continue;
    }
    let end = index;
    while (end < ops.length && ops[end].kind === "same") {
      end += 1;
    }
    const keepBefore = index === 0 ? 0 : CONTEXT_LINES;
    const keepAfter = end === ops.length ? 0 : CONTEXT_LINES;
    const run = end - index;
    if (run > keepBefore + keepAfter + 1) {
      out.push(...ops.slice(index, index + keepBefore));
      out.push({ kind: "skip", count: run - keepBefore - keepAfter });
      out.push(...ops.slice(end - keepAfter, end));
    } else {
      out.push(...ops.slice(index, end));
    }
    index = end;
  }
  return out;
}
