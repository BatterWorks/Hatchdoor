import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import {
  browsableVault,
  conflictVault,
  healthyVault,
  indexingVault,
  readOnlyVault,
  staleVault,
  syncFailedVault,
  syncStoppedVault,
  unavailableVault,
} from "../test/fixtures/vaults";
import type { VaultSummary } from "../types";
import { VaultAggregateSlot, VaultSlot } from "./vaultSlot";
import {
  deriveVaultAggregate,
  deriveVaultSlot,
  describeScopeSlot,
} from "./vaultSlotLogic";

afterEach(cleanup);

describe("deriveVaultSlot", () => {
  it("shows the note count for a healthy Vault", () => {
    expect(deriveVaultSlot(healthyVault("Alpha"), 126)).toEqual({
      kind: "count",
      count: 126,
    });
  });

  it("shows the note count for a read-only Vault — the slot carries conditions, never configuration", () => {
    expect(deriveVaultSlot(readOnlyVault("Alpha"), 12)).toEqual({
      kind: "count",
      count: 12,
    });
  });

  it("shows the note count, not a placeholder, for a browsable Vault still building search", () => {
    expect(deriveVaultSlot(browsableVault("Alpha"), 126)).toEqual({
      kind: "count-pending-search",
      count: 126,
      sentence: "Browsing is ready. Search for this Vault is still building.",
    });
  });

  it("reports a failed embedding pass instead of claiming search is still building", () => {
    const vault = browsableVault("Alpha");
    expect(
      deriveVaultSlot(
        {
          ...vault,
          search_error: {
            code: "vault_index_failed",
            message: "Embedding ran out of memory.",
            retryable: true,
          },
        },
        126,
      ),
    ).toEqual({
      kind: "condition",
      word: "search failed",
      tier: "warn",
      sentence: "Embedding ran out of memory.",
    });
  });

  it("counts a browsable Vault as participating — it has published Notes to read", () => {
    const vault = browsableVault("Alpha");
    expect(
      deriveVaultAggregate([vault], { [vault.vault_id]: 126 }),
    ).toEqual({ kind: "count", count: 1 });
  });

  it("announces a browsable Vault's count and its pending search", () => {
    const vault = browsableVault("Alpha");
    expect(
      describeScopeSlot(vault.vault_id, [vault], { [vault.vault_id]: 126 }),
    ).toBe("126 notes, search still building");
  });

  it("shows indexing for a Vault mid-index", () => {
    expect(deriveVaultSlot(indexingVault("Alpha"), 40)).toEqual({
      kind: "indexing",
    });
  });

  it("shows indexing, not unavailable, for a Vault that has never published a snapshot", () => {
    // activation stays "active" the moment a Vault's directory stats fine,
    // regardless of whether it has indexed yet — only a genuinely dead
    // runtime turns activation "unavailable" too (src/vault_runtime.rs's
    // activation_snapshot). This is the signal #116 says the client already
    // has to tell "first index" apart from "lost it".
    const neverIndexed: VaultSummary = {
      ...healthyVault("Alpha"),
      search: "unavailable",
    };
    expect(deriveVaultSlot(neverIndexed, undefined)).toEqual({
      kind: "indexing",
    });
  });

  it("shows stale in warn tier when indexing has failed", () => {
    const result = deriveVaultSlot(staleVault("Alpha"), 40);
    expect(result).toMatchObject({
      kind: "condition",
      word: "stale",
      tier: "warn",
    });
    if (result.kind === "condition") {
      expect(result.sentence).toBe(
        "The last index build for this Vault failed.",
      );
    }
  });

  it("shows sync failed in warn tier for a retryable Git failure", () => {
    const result = deriveVaultSlot(syncFailedVault("Alpha"), 40);
    expect(result).toMatchObject({
      kind: "condition",
      word: "sync failed",
      tier: "warn",
    });
  });

  it("shows sync stopped in error tier for dirty_working_copy", () => {
    const result = deriveVaultSlot(syncStoppedVault("Alpha"), 40);
    expect(result).toMatchObject({
      kind: "condition",
      word: "sync stopped",
      tier: "error",
    });
  });

  it("shows conflict in error tier for git_content_conflict", () => {
    const result = deriveVaultSlot(conflictVault("Alpha"), 40);
    expect(result).toMatchObject({
      kind: "condition",
      word: "conflict",
      tier: "error",
    });
  });

  it("shows unavailable in error tier when the runtime is genuinely down", () => {
    const result = deriveVaultSlot(unavailableVault("Alpha"), undefined);
    expect(result).toMatchObject({
      kind: "condition",
      word: "unavailable",
      tier: "error",
    });
  });

  it("ranks unavailable above every other condition", () => {
    const vault: VaultSummary = {
      ...conflictVault("Alpha"),
      activation: "unavailable",
    };
    expect(deriveVaultSlot(vault, undefined)).toMatchObject({
      word: "unavailable",
    });
  });

  it("ranks a Git condition above a stale search condition", () => {
    const vault: VaultSummary = {
      ...conflictVault("Alpha"),
      search: "stale",
    };
    expect(deriveVaultSlot(vault, undefined)).toMatchObject({
      word: "conflict",
    });
  });

  it("clamps every condition to the amber tier in demo mode (#152)", () => {
    for (const vault of [
      unavailableVault("Alpha"),
      conflictVault("Beta"),
      syncStoppedVault("Gamma"),
    ]) {
      expect(deriveVaultSlot(vault, undefined, true)).toMatchObject({
        tier: "warn",
      });
    }
    // Already amber outside demo mode — stays amber, unaffected.
    expect(deriveVaultSlot(staleVault("Delta"), undefined, true)).toMatchObject(
      { tier: "warn" },
    );
  });

  it("shows the instruction-free fallback sentence, not the Vault's own runtime message, in demo mode (#152)", () => {
    const vault: VaultSummary = {
      ...unavailableVault("Alpha"),
      activation_error: {
        code: "vault_unavailable",
        message: "Run `hatchdoor vault repair` on the operator console.",
        retryable: true,
      },
    };
    const result = deriveVaultSlot(vault, undefined, true);
    expect(result).toMatchObject({
      tier: "warn",
      sentence: "This Vault should be answering and is not.",
    });
  });
});

describe("VaultSlot", () => {
  it("renders the count as plain, muted mono text", () => {
    render(<VaultSlot vault={healthyVault("Alpha")} noteCount={126} />);

    const count = screen.getByText("126");
    expect(count).toHaveClass("side-count");
  });

  it("renders a shimmering placeholder while indexing, with no word", () => {
    render(<VaultSlot vault={indexingVault("Alpha")} noteCount={undefined} />);

    expect(
      screen.getByRole("status", { name: "Indexing" }),
    ).toBeInTheDocument();
  });

  it("distinguishes the red tier by form (a class), not colour alone", () => {
    render(<VaultSlot vault={conflictVault("Alpha")} noteCount={undefined} />);

    const slot = screen.getByText("conflict");
    expect(slot).toHaveClass("vault-slot-condition");
    expect(slot).toHaveClass("vault-tier-error");
  });

  it("keeps the amber tier plain, without the red tier's form class", () => {
    render(<VaultSlot vault={staleVault("Alpha")} noteCount={undefined} />);

    const slot = screen.getByText("stale");
    expect(slot).toHaveClass("vault-tier-warn");
    expect(slot).not.toHaveClass("vault-tier-error");
  });

  it("carries the plain-language sentence in hover text and accessible name, while the word on screen stays terse", () => {
    render(
      <VaultSlot vault={syncStoppedVault("Alpha")} noteCount={undefined} />,
    );

    const slot = screen.getByText("sync stopped");
    expect(slot).toHaveAttribute(
      "title",
      "Local edits in this Vault halted Git integration.",
    );
    expect(slot).toHaveAttribute(
      "aria-label",
      "Local edits in this Vault halted Git integration.",
    );
  });

  it("renders a red-tier condition in the amber tier in demo mode, never bordered-ground red (#152)", () => {
    render(
      <VaultSlot
        vault={conflictVault("Alpha")}
        noteCount={undefined}
        demoMode
      />,
    );

    const slot = screen.getByText("conflict");
    expect(slot).toHaveClass("vault-tier-warn");
    expect(slot).not.toHaveClass("vault-tier-error");
  });
});

describe("deriveVaultAggregate", () => {
  const alpha = healthyVault("Alpha");
  const beta = healthyVault("Beta");
  const gamma = healthyVault("Gamma");

  it("is the plain Vault count when every Vault is healthy", () => {
    expect(deriveVaultAggregate([alpha, beta, gamma], {})).toEqual({
      kind: "count",
      count: 3,
    });
  });

  it("still counts an indexing Vault as participating — moving is not stuck", () => {
    const indexing = indexingVault("Beta");
    expect(deriveVaultAggregate([alpha, indexing, gamma], {})).toEqual({
      kind: "count",
      count: 3,
    });
  });

  it("shows the shortfall in warn tier when only amber conditions are present", () => {
    const stale = staleVault("Beta");
    expect(deriveVaultAggregate([alpha, stale, gamma], {})).toEqual({
      kind: "shortfall",
      participating: 2,
      total: 3,
      tier: "warn",
    });
  });

  it("shows the shortfall in error tier when any red condition is present, even alongside amber", () => {
    const stale = staleVault("Beta");
    const unavailable = unavailableVault("Gamma");
    expect(deriveVaultAggregate([alpha, stale, unavailable], {})).toEqual({
      kind: "shortfall",
      participating: 1,
      total: 3,
      tier: "error",
    });
  });

  it("never reports the error tier in demo mode, even with an unavailable Vault present (#152)", () => {
    const unavailable = unavailableVault("Gamma");
    expect(deriveVaultAggregate([alpha, unavailable], {}, true)).toEqual({
      kind: "shortfall",
      participating: 1,
      total: 2,
      tier: "warn",
    });
  });
});

describe("describeScopeSlot", () => {
  const alpha = healthyVault("Alpha");
  const beta = healthyVault("Beta");

  it("names the plain Vault count at all scope", () => {
    expect(describeScopeSlot("all", [alpha, beta], {})).toBe("2 Vaults");
  });

  it("uses the singular for exactly one Vault", () => {
    expect(describeScopeSlot("all", [alpha], {})).toBe("1 Vault");
  });

  it("names the shortfall in the slot's own words at all scope", () => {
    const unavailable = unavailableVault("Beta");
    expect(describeScopeSlot("all", [alpha, unavailable], {})).toBe("1 of 2");
  });

  it("names a healthy Vault's note count when narrowed", () => {
    expect(
      describeScopeSlot(alpha.vault_id, [alpha, beta], {
        [alpha.vault_id]: 42,
      }),
    ).toBe("42 notes");
  });

  it("uses the singular for exactly one note", () => {
    expect(
      describeScopeSlot(alpha.vault_id, [alpha], { [alpha.vault_id]: 1 }),
    ).toBe("1 note");
  });

  it("names a condition word when narrowed to a Vault in trouble", () => {
    const stale = staleVault("Alpha");
    expect(describeScopeSlot(stale.vault_id, [stale], {})).toBe("stale");
  });

  it("is unknown while the narrowed Vault is still indexing", () => {
    const indexing = indexingVault("Alpha");
    expect(describeScopeSlot(indexing.vault_id, [indexing], {})).toBeNull();
  });
});

describe("VaultAggregateSlot", () => {
  it("renders the plain count when nothing is wrong", () => {
    render(
      <VaultAggregateSlot
        vaults={[healthyVault("Alpha"), healthyVault("Beta")]}
        counts={{}}
      />,
    );

    expect(screen.getByText("2")).toHaveClass("side-count");
  });

  it("renders the shortfall reading with the tier's form class", () => {
    render(
      <VaultAggregateSlot
        vaults={[healthyVault("Alpha"), unavailableVault("Beta")]}
        counts={{}}
      />,
    );

    const shortfall = screen.getByText("1 of 2");
    expect(shortfall).toHaveClass("vault-slot-shortfall");
    expect(shortfall).toHaveClass("vault-tier-error");
  });

  it("clamps the shortfall reading to amber in demo mode (#152)", () => {
    render(
      <VaultAggregateSlot
        vaults={[healthyVault("Alpha"), unavailableVault("Beta")]}
        counts={{}}
        demoMode
      />,
    );

    const shortfall = screen.getByText("1 of 2");
    expect(shortfall).toHaveClass("vault-tier-warn");
    expect(shortfall).not.toHaveClass("vault-tier-error");
  });
});
