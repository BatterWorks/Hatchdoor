import { cleanup, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, describe, expect, it, vi } from "vitest";

import { StatsPage } from "./StatsPage";
import { VAULT_SCOPE_KEY } from "../app/constants";
import { discoveryResponse, healthyVault } from "../test/fixtures/vaults";
import type { MonthActivity, VaultStats, VaultSummary } from "../types";

const FIRST = healthyVault("Notes");
const SECOND = healthyVault("Archive");

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status });
}

/**
 * The six-entry activity window the backend now guarantees: oldest first, one
 * entry per calendar month, zeros included.
 */
function activityWindow(counts: number[]): MonthActivity[] {
  const months = [
    "2026-04",
    "2026-05",
    "2026-06",
    "2026-07",
    "2026-08",
    "2026-09",
  ];
  return months.map((month, index) => ({
    month,
    modified_count: counts[index],
  }));
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
  activity: MonthActivity[] = [],
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
          stats: {
            ...statsWithNoteCount(counts[vaultId] ?? 0),
            activity_by_month: activity,
          },
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

/**
 * Renders one Vault whose only distinguishing figure is its note count, with
 * the supplied activity window, and waits for that count to appear.
 */
async function renderActivity(activity: MonthActivity[]) {
  window.localStorage.setItem(VAULT_SCOPE_KEY, FIRST.vault_id);
  mockInstance([FIRST], { [FIRST.vault_id]: 11 }, [], activity);

  const rendered = renderStats();
  await waitFor(() => expect(screen.getByText("11")).toBeTruthy());
  return rendered;
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

describe("Writing Activity reads as a calendar (#298)", () => {
  it("draws the supplied window in order and averages over six months", async () => {
    const { container } = await renderActivity(
      activityWindow([0, 0, 1, 0, 2, 3]),
    );

    const labels = Array.from(
      container.querySelectorAll(".stats-act-month"),
    ).map((node) => node.textContent);
    expect(labels).toEqual(["Apr", "May", "Jun", "Jul", "Aug", "Sep"]);

    // Three of the six months are empty and still hold a column of their own,
    // so the axis stays a timeline rather than a list of months with notes.
    const counts = Array.from(
      container.querySelectorAll(".stats-act-count"),
    ).map((node) => node.textContent);
    expect(counts).toEqual(["0", "0", "1", "0", "2", "3"]);

    expect(screen.getByText("Avg: 1.0 / month")).toBeTruthy();
    expect(screen.getByText(/Peak: Sep · 3 notes/)).toBeTruthy();
  });

  it("draws finite bars and a peak for a window nobody wrote in", async () => {
    const { container } = await renderActivity(
      activityWindow([0, 0, 0, 0, 0, 0]),
    );

    const bars = Array.from(
      container.querySelectorAll<HTMLElement>(".stats-act-bar"),
    );
    expect(bars).toHaveLength(6);
    // Scaling against a zero maximum would ask for a NaN height.
    expect(bars.map((bar) => bar.style.height)).toEqual(Array(6).fill("2px"));
    expect(screen.getByText(/Peak: Apr · 0 notes/)).toBeTruthy();
    expect(screen.getByText("Avg: 0.0 / month")).toBeTruthy();
  });

  it("averages over the window rather than over the bars it received", async () => {
    const { container } = await renderActivity([
      { month: "2026-08", modified_count: 6 },
      { month: "2026-09", modified_count: 6 },
    ]);

    expect(container.querySelectorAll(".stats-act-col")).toHaveLength(2);
    // Twelve notes over a six-month window is 2.0, not 6.0 over two bars.
    expect(screen.getByText("Avg: 2.0 / month")).toBeTruthy();
  });
});
