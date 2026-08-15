import { cleanup, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, describe, expect, it, vi } from "vitest";

import { StatsPage } from "./StatsPage";
import { VAULT_SCOPE_KEY } from "../app/constants";
import { discoveryResponse, healthyVault } from "../test/fixtures/vaults";
import type { VaultStats, VaultSummary } from "../types";

const FIRST = healthyVault("Notes");
const SECOND = healthyVault("Archive");

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status });
}

/** A complete `VaultStats` whose note count identifies which Vault answered. */
function statsWithNoteCount(noteCount: number): VaultStats {
  return {
    note_count: noteCount,
    word_count: 0,
    tag_count: 0,
    link_count: 0,
    image_count: 0,
    top_tags: [],
    most_linked: [],
    activity_by_month: [],
    notes_per_folder: [],
    longest_notes: [],
    shortest_notes: [],
    total_outgoing_links: 0,
    total_backlinks: 0,
    avg_word_count: 0,
    vault_size_bytes: 0,
    modified_this_week: { count: 0, notes: [] },
    modified_this_month: { count: 0, notes: [] },
    orphan_notes: [],
    no_tag_notes: [],
  };
}

/**
 * Serves discovery plus one `stats/detail` per Vault, each answering with a
 * distinct note count so a test can prove *which* Vault's numbers rendered.
 * `failing` names Vaults whose detail read 503s, standing in for a Vault that
 * cannot answer while its neighbours can.
 */
function mockInstance(
  vaults: VaultSummary[],
  counts: Record<string, number>,
  failing: string[] = [],
) {
  return vi
    .spyOn(globalThis, "fetch")
    .mockImplementation(async (input: RequestInfo | URL) => {
      const url = String(input);

      if (url.endsWith("/api/v1/vaults")) {
        return jsonResponse(discoveryResponse(vaults, false));
      }

      const detail = /\/api\/v1\/vaults\/([^/]+)\/stats\/detail/.exec(url);
      if (detail) {
        const vaultId = decodeURIComponent(detail[1]);
        if (failing.includes(vaultId)) {
          return jsonResponse(
            {
              code: "vault_unavailable",
              message: "This Vault is not available.",
              retryable: true,
            },
            503,
          );
        }
        const vault = vaults.find(
          (candidate) => candidate.vault_id === vaultId,
        );
        return jsonResponse({
          vault_id: vaultId,
          vault_name: vault?.name ?? "",
          stats: statsWithNoteCount(counts[vaultId] ?? 0),
        });
      }

      throw new Error(`unexpected fetch: ${url}`);
    });
}

function renderStats() {
  return render(
    <MemoryRouter>
      <StatsPage />
    </MemoryRouter>,
  );
}

afterEach(() => {
  cleanup();
  window.localStorage.clear();
  vi.restoreAllMocks();
});

describe("StatsPage honours Vault scope (#102)", () => {
  it("reads the narrowed Vault, not the first enabled one", async () => {
    window.localStorage.setItem(VAULT_SCOPE_KEY, SECOND.vault_id);
    const fetchMock = mockInstance([FIRST, SECOND], {
      [FIRST.vault_id]: 11,
      [SECOND.vault_id]: 22,
    });

    renderStats();

    await waitFor(() => expect(screen.getByText("22")).toBeTruthy());
    expect(screen.queryByText("11")).toBeNull();

    const detailUrls = fetchMock.mock.calls
      .map((call) => String(call[0]))
      .filter((url) => url.includes("/stats/detail"));
    expect(detailUrls).toHaveLength(1);
    expect(detailUrls[0]).toContain(encodeURIComponent(SECOND.vault_id));
  });

  it("renders one titled section per enabled Vault at `all`", async () => {
    window.localStorage.setItem(VAULT_SCOPE_KEY, "all");
    mockInstance([FIRST, SECOND], {
      [FIRST.vault_id]: 11,
      [SECOND.vault_id]: 22,
    });

    renderStats();

    await waitFor(() => expect(screen.getByText("11")).toBeTruthy());
    expect(screen.getByText("22")).toBeTruthy();

    const headings = screen
      .getAllByRole("heading", { level: 2 })
      .map((node) => node.textContent);
    expect(headings).toEqual(["Notes", "Archive"]);
  });

  it("keeps a single-Vault instance free of per-Vault section chrome", async () => {
    window.localStorage.setItem(VAULT_SCOPE_KEY, "all");
    mockInstance([FIRST], { [FIRST.vault_id]: 11 });

    renderStats();

    await waitFor(() => expect(screen.getByText("11")).toBeTruthy());
    expect(screen.queryByRole("heading", { level: 2 })).toBeNull();
  });

  it("states an unavailable Vault while its neighbours still render", async () => {
    window.localStorage.setItem(VAULT_SCOPE_KEY, "all");
    mockInstance(
      [FIRST, SECOND],
      { [FIRST.vault_id]: 11, [SECOND.vault_id]: 22 },
      [SECOND.vault_id],
    );

    renderStats();

    await waitFor(() => expect(screen.getByText("11")).toBeTruthy());
    expect(screen.getByText(/This Vault is not available\./)).toBeTruthy();
    expect(screen.queryByText("22")).toBeNull();
  });
});
