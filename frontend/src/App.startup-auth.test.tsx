import { act, cleanup, render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, expect, it, vi } from "vitest";

import { App } from "./App";
import { clearToken } from "./api/api";

afterEach(() => {
  cleanup();
  clearToken();
  vi.restoreAllMocks();
});

it("prompts for the web token when first-run model setup is unauthorized", async () => {
  vi.spyOn(globalThis, "fetch")
    .mockResolvedValueOnce(
      new Response(JSON.stringify({ state: "terms_required" }), { status: 200 }),
    )
    .mockResolvedValueOnce(new Response(null, { status: 401 }));

  render(
    <MemoryRouter>
      <App />
    </MemoryRouter>,
  );

  await screen.findByRole("button", { name: "Accept terms and set up Gemma" });
  await act(async () => {
    screen.getByRole("button", { name: "Accept terms and set up Gemma" }).click();
  });

  expect(await screen.findByRole("dialog", { name: "Access token required" })).toBeVisible();
});
