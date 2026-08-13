import { describe, expect, it } from "vitest";

import type { VaultParticipant } from "../types";
import { describeMissingVaults, missingVaultNames } from "./vaultParticipants";

function participant(
  name: string,
  state: VaultParticipant["state"],
): VaultParticipant {
  return { vault_id: name.toLowerCase(), vault_name: name, state };
}

describe("missingVaultNames", () => {
  it("names only participants that did not come back fresh", () => {
    const participants = [
      participant("Work", "fresh"),
      participant("Personal", "fresh"),
      participant("Archive", "unavailable"),
    ];
    expect(missingVaultNames(participants)).toEqual(["Archive"]);
  });

  it("includes a stale participant, not just unavailable ones", () => {
    const participants = [participant("Archive", "stale")];
    expect(missingVaultNames(participants)).toEqual(["Archive"]);
  });

  it("is empty when every participant is fresh", () => {
    const participants = [
      participant("Work", "fresh"),
      participant("Personal", "fresh"),
    ];
    expect(missingVaultNames(participants)).toEqual([]);
  });
});

describe("describeMissingVaults", () => {
  it("names one Vault", () => {
    expect(describeMissingVaults(["Archive"])).toBe("Archive did not answer.");
  });

  it("joins two Vaults with 'and'", () => {
    expect(describeMissingVaults(["Archive", "Journal"])).toBe(
      "Archive and Journal did not answer.",
    );
  });

  it("joins three or more Vaults with a serial comma", () => {
    expect(
      describeMissingVaults(["Archive", "Journal", "Scratch", "Vault Four"]),
    ).toBe("Archive, Journal, Scratch, and Vault Four did not answer.");
  });
});
