import { afterEach, beforeEach, describe, expect, it } from "vitest";

import {
  EIGHT_VAULTS,
  healthyVault,
  THREE_VAULTS,
  unavailableVault,
} from "../test/fixtures/vaults";
import {
  expandedFoldersForVault,
  getStoredUnfoldedVault,
  isVaultUnfoldable,
  resolveInitialUnfoldedVault,
  resolveLandingVaultId,
  setStoredUnfoldedVault,
  withVaultFolderChange,
} from "./vaultAccordion";

describe("isVaultUnfoldable", () => {
  it("is unfoldable for every activation but unavailable", () => {
    expect(isVaultUnfoldable(healthyVault("Alpha"))).toBe(true);
  });

  it("is not unfoldable when the Vault cannot answer", () => {
    expect(isVaultUnfoldable(unavailableVault())).toBe(false);
  });
});

describe("resolveInitialUnfoldedVault", () => {
  it("unfolds the landing Vault when one is given and it can unfold", () => {
    const landing = THREE_VAULTS[1].vault_id;
    expect(
      resolveInitialUnfoldedVault(landing, null, THREE_VAULTS),
    ).toBe(landing);
  });

  it("falls back to the stored Vault when there is no landing Vault", () => {
    const stored = THREE_VAULTS[2].vault_id;
    expect(
      resolveInitialUnfoldedVault(undefined, stored, THREE_VAULTS),
    ).toBe(stored);
  });

  it("unfolds nothing when neither a landing nor a stored Vault exists", () => {
    expect(
      resolveInitialUnfoldedVault(undefined, null, THREE_VAULTS),
    ).toBeUndefined();
  });

  it("never unfolds an unavailable Vault, even as the landing Vault", () => {
    const vaults = [THREE_VAULTS[0], unavailableVault("Down")];
    expect(
      resolveInitialUnfoldedVault(vaults[1].vault_id, null, vaults),
    ).toBeUndefined();
  });

  it("never unfolds an unavailable Vault, even as the stored Vault", () => {
    const vaults = [THREE_VAULTS[0], unavailableVault("Down")];
    expect(
      resolveInitialUnfoldedVault(undefined, vaults[1].vault_id, vaults),
    ).toBeUndefined();
  });

  it("ignores a stored Vault id that no longer exists", () => {
    expect(
      resolveInitialUnfoldedVault(undefined, "ghost-vault", THREE_VAULTS),
    ).toBeUndefined();
  });
});

describe("resolveLandingVaultId", () => {
  afterEach(() => localStorage.clear());

  it("reads the Vault directly off a note route", () => {
    expect(
      resolveLandingVaultId(`/v/${THREE_VAULTS[0].vault_id}/n/some-note`),
    ).toBe(THREE_VAULTS[0].vault_id);
  });

  it("falls back to the stored last note's Vault when landing at the root", () => {
    localStorage.setItem(
      "hatchdoor.lastNote",
      JSON.stringify({ vaultId: THREE_VAULTS[1].vault_id, slug: "home" }),
    );
    expect(resolveLandingVaultId("/")).toBe(THREE_VAULTS[1].vault_id);
  });

  it("resolves nothing at the root with no stored last note", () => {
    expect(resolveLandingVaultId("/")).toBeUndefined();
  });

  it("resolves nothing on a non-note, non-root route", () => {
    expect(resolveLandingVaultId("/graph")).toBeUndefined();
  });
});

describe("stored unfolded Vault", () => {
  beforeEach(() => localStorage.clear());
  afterEach(() => localStorage.clear());

  it("round-trips a Vault id", () => {
    setStoredUnfoldedVault(THREE_VAULTS[0].vault_id);
    expect(getStoredUnfoldedVault()).toBe(THREE_VAULTS[0].vault_id);
  });

  it("returns null when nothing is stored", () => {
    expect(getStoredUnfoldedVault()).toBeNull();
  });

  it("clears the stored Vault when set to null", () => {
    setStoredUnfoldedVault(THREE_VAULTS[0].vault_id);
    setStoredUnfoldedVault(null);
    expect(getStoredUnfoldedVault()).toBeNull();
  });
});

describe("per-Vault folder-open namespacing", () => {
  const vaultA = EIGHT_VAULTS[0].vault_id;
  const vaultB = EIGHT_VAULTS[1].vault_id;

  it("reads back only the given Vault's own folder-open entries", () => {
    const stored = withVaultFolderChange({}, vaultA, { "10-journal": true });
    const withB = withVaultFolderChange(stored, vaultB, { "10-journal": false });

    expect(expandedFoldersForVault(withB, vaultA)).toEqual({
      "10-journal": true,
    });
    expect(expandedFoldersForVault(withB, vaultB)).toEqual({
      "10-journal": false,
    });
  });

  it("keeps two Vaults' identically-named folders independent", () => {
    let stored = withVaultFolderChange({}, vaultA, { Journal: true });
    stored = withVaultFolderChange(stored, vaultB, {});

    expect(expandedFoldersForVault(stored, vaultB)).toEqual({});
  });

  it("preserves a Vault's remembered state when another Vault's state changes", () => {
    let stored = withVaultFolderChange({}, vaultA, { Journal: true });
    stored = withVaultFolderChange(stored, vaultB, { Reading: true });

    expect(expandedFoldersForVault(stored, vaultA)).toEqual({ Journal: true });
  });
});
