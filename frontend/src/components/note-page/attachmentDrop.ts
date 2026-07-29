// Attachment handling shared by the note editor and the note body drop target.

import { normalizeImageForUpload } from "../../lib/imageUpload";
import type { AttachmentOutcome } from "../../types";

const ATTACHMENT_FOLDER = "Attachments";

// Mirrors allowed_attachment_extensions() in src/vault/write/paths.rs. Without
// this the browser happily uploads a .docx and the user sees a raw 400.
export const ATTACHMENT_EXTENSIONS = [
  "png",
  "jpg",
  "jpeg",
  "gif",
  "webp",
  "avif",
  "bmp",
  "pdf",
] as const;

// Mirrors DEFAULT_MAX_ATTACHMENT_BYTES in src/mcp/config.rs. The server stays
// authoritative; this only buys a useful message instead of a failed request.
export const ATTACHMENT_MAX_BYTES = 10 * 1024 * 1024;

/**
 * Why this file cannot be attached, or null if it can.
 */
export function attachmentRejection(file: File): string | null {
  const extension = file.name.split(".").pop()?.toLowerCase() ?? "";
  const hasExtension = file.name.includes(".");

  if (
    !hasExtension ||
    !(ATTACHMENT_EXTENSIONS as readonly string[]).includes(extension)
  ) {
    return "Hatchdoor accepts images and PDFs.";
  }

  if (file.size > ATTACHMENT_MAX_BYTES) {
    return `That file is ${formatMegabytes(file.size)}. The limit is ${formatMegabytes(
      ATTACHMENT_MAX_BYTES,
    )}.`;
  }

  return null;
}

function formatMegabytes(bytes: number): string {
  const mb = bytes / (1024 * 1024);
  const rounded = mb >= 10 ? Math.round(mb) : Math.round(mb * 10) / 10;
  return `${rounded} MB`;
}

export type UploadAttachmentFn = (
  file: File,
  targetRelativePath: string,
) => Promise<AttachmentOutcome>;

export type NoteAttachmentUpload = {
  embedPath: string;
  gitSyncWarning?: string;
};

/**
 * Upload a file into the vault's Attachments folder and report the path to
 * embed in `noteRelativePath`.
 */
export async function uploadNoteAttachment(
  file: File,
  noteRelativePath: string,
  upload: UploadAttachmentFn,
): Promise<NoteAttachmentUpload> {
  const normalized = await normalizeImageForUpload(file);
  const filename = safeAttachmentFilename(normalized.name);

  // The server refuses to overwrite an existing attachment, so a second
  // report.pdf comes back 409. Count up rather than making the user rename it.
  let attempt = 0;
  for (;;) {
    const candidate = numberedFilename(filename, attempt);
    try {
      const outcome = await upload(
        normalized,
        `${ATTACHMENT_FOLDER}/${candidate}`,
      );
      return {
        embedPath: attachmentEmbedPath(
          outcome.attachment.relative_path,
          noteRelativePath,
        ),
        gitSyncWarning: outcome.git_sync_warning ?? undefined,
      };
    } catch (error) {
      attempt += 1;
      const isConflict =
        error instanceof Error && error.name === "ConflictError";
      if (!isConflict || attempt > MAX_FILENAME_ATTEMPTS) {
        throw error;
      }
    }
  }
}

const MAX_FILENAME_ATTEMPTS = 50;

function numberedFilename(filename: string, attempt: number): string {
  if (attempt === 0) {
    return filename;
  }
  const dot = filename.lastIndexOf(".");
  if (dot <= 0) {
    return `${filename}-${attempt}`;
  }
  return `${filename.slice(0, dot)}-${attempt}${filename.slice(dot)}`;
}

export function safeAttachmentFilename(filename: string): string {
  const basename = filename.split(/[\\/]/).pop()?.trim() || "attachment";
  return basename.replace(/[^A-Za-z0-9._ -]/g, "-");
}

/**
 * The embed path to write into a note for an attachment stored at
 * `vaultRelativePath`.
 *
 * Embeds resolve relative to the note's own directory, so a note outside the
 * vault root needs a path that walks back out to the root first.
 */
export function attachmentEmbedPath(
  vaultRelativePath: string,
  noteRelativePath: string,
): string {
  const depth = noteRelativePath
    .split("/")
    .filter((part) => part.length > 0).length;
  return "../".repeat(Math.max(depth - 1, 0)) + vaultRelativePath;
}
