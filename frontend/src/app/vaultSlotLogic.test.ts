import { describe, expect, it } from "vitest";

import { healthyVault } from "../test/fixtures/vaults";
import type { VaultSummary } from "../types";
import { noteInSyncConflict } from "./vaultSlotLogic";

function conflicted(
  paths: string[],
  source?: VaultSummary["source"],
): VaultSummary {
  return healthyVault("Conflicted", {
    source,
    git: "unavailable",
    git_error: {
      code: "managed_git_conflict",
      message: "managed checkout merge conflict",
      retryable: false,
      detail: { kind: "affected_paths", paths, total: paths.length },
    },
  });
}

describe("noteInSyncConflict (ADR-30)", () => {
  it("matches a note by its file path, extension included", () => {
    const vault = conflicted(["Groceries.md", "Plans/Trip.md"]);
    expect(noteInSyncConflict(vault, "Groceries")).toBe(true);
    expect(noteInSyncConflict(vault, "Plans/Trip")).toBe(true);
    expect(noteInSyncConflict(vault, "Plans/Other")).toBe(false);
  });

  it("puts the Vault's folder in front for a Vault kept inside its repository", () => {
    const vault = conflicted(["notes/Groceries.md"], {
      type: "managed_git",
      repository_url: "https://example.test/notes.git",
      vault_subdirectory: "notes/",
      mode: "two_way",
      poll_interval_secs: 3600,
    });
    expect(noteInSyncConflict(vault, "Groceries")).toBe(true);
    expect(
      noteInSyncConflict(conflicted(["notes/Groceries.md"]), "Groceries"),
    ).toBe(false);
  });

  it("says nothing for any other Git failure or a healthy Vault", () => {
    const other = conflicted(["Groceries.md"]);
    other.git_error = {
      ...other.git_error!,
      code: "managed_git_dirty_working_copy",
    };
    expect(noteInSyncConflict(other, "Groceries")).toBe(false);
    expect(noteInSyncConflict(healthyVault("Fine"), "Groceries")).toBe(false);
    expect(noteInSyncConflict(undefined, "Groceries")).toBe(false);
  });
});
