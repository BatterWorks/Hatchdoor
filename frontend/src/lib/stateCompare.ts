import type { ExplorerFolder, ExplorerNote, Note, NoteLinks } from "../types";

export function isNoteEqual(left: Note | null, right: Note | null): boolean {
  if (left === right) {
    return true;
  }
  if (!left || !right) {
    return false;
  }

  return (
    left.title === right.title &&
    left.slug === right.slug &&
    left.relative_path === right.relative_path &&
    left.content === right.content
  );
}

export function isExplorerTreeEqual(
  left: ExplorerFolder | null,
  right: ExplorerFolder | null,
): boolean {
  if (left === right) {
    return true;
  }
  if (!left || !right) {
    return false;
  }

  if (left.name !== right.name) {
    return false;
  }

  if (!isExplorerNotesEqual(left.notes, right.notes)) {
    return false;
  }

  if (left.folders.length !== right.folders.length) {
    return false;
  }

  for (let i = 0; i < left.folders.length; i += 1) {
    if (!isExplorerTreeEqual(left.folders[i], right.folders[i])) {
      return false;
    }
  }

  return true;
}

export function isNoteLinksEqual(
  left: NoteLinks | null,
  right: NoteLinks | null,
): boolean {
  if (left === right) {
    return true;
  }
  if (!left || !right) {
    return false;
  }

  return (
    isNoteLinksListEqual(left.outgoing, right.outgoing) &&
    isNoteLinksListEqual(left.backlinks, right.backlinks)
  );
}

function isExplorerNotesEqual(
  left: ExplorerNote[],
  right: ExplorerNote[],
): boolean {
  if (left.length !== right.length) {
    return false;
  }

  for (let i = 0; i < left.length; i += 1) {
    if (left[i].slug !== right[i].slug || left[i].title !== right[i].title) {
      return false;
    }
  }

  return true;
}

function isNoteLinksListEqual(
  left: NoteLinks["outgoing"],
  right: NoteLinks["outgoing"],
): boolean {
  if (left.length !== right.length) {
    return false;
  }

  for (let i = 0; i < left.length; i += 1) {
    if (
      left[i].slug !== right[i].slug ||
      left[i].title !== right[i].title ||
      left[i].relative_path !== right[i].relative_path
    ) {
      return false;
    }
  }

  return true;
}
