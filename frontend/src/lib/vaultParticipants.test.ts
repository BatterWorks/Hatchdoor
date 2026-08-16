import { describe, expect, it } from "vitest";

import type { VaultParticipant } from "../types";
import {
  describeMissingVaults,
  describeNotSearchableVaults,
  describeVaultsNotDrawn,
  missingVaultNames,
  notSearchableVaultNames,
} from "./vaultParticipants";

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

describe("not-searchable participants", () => {
  it("does not call a Vault that is still building search one that did not answer", () => {
    const participants = [
      participant("Work", "fresh"),
      participant("Personal", "not_searchable"),
      participant("Archive", "unavailable"),
    ];
    expect(missingVaultNames(participants)).toEqual(["Archive"]);
    expect(notSearchableVaultNames(participants)).toEqual(["Personal"]);
  });

  it("says one Vault is still building search", () => {
    expect(describeNotSearchableVaults(["Personal"])).toBe(
      "Personal is still building search.",
    );
  });

  it("says several Vaults are still building search", () => {
    expect(describeNotSearchableVaults(["Personal", "Work"])).toBe(
      "Personal and Work are still building search.",
    );
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

describe("describeVaultsNotDrawn", () => {
  it("names one Vault", () => {
    expect(describeVaultsNotDrawn(["Field Station"])).toBe(
      "Field Station could not be drawn.",
    );
  });

  it("joins two Vaults with 'and'", () => {
    expect(describeVaultsNotDrawn(["Field Station", "Archive"])).toBe(
      "Field Station and Archive could not be drawn.",
    );
  });

  it("joins three or more Vaults with a serial comma", () => {
    expect(
      describeVaultsNotDrawn(["Field Station", "Archive", "Journal"]),
    ).toBe("Field Station, Archive, and Journal could not be drawn.");
  });
});
