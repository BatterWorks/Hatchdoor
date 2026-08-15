import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { apiFetch } from "../../api/api";
import { VaultCreationDialog } from "./VaultCreation";

vi.mock("../../api/api", () => ({ apiFetch: vi.fn() }));
const mockedApiFetch = vi.mocked(apiFetch);

const json = (body: unknown, init?: ResponseInit) =>
  new Response(JSON.stringify(body), {
    headers: { "content-type": "application/json" },
    ...init,
  });

function mockRoutes(
  routes: Record<
    string,
    (init: RequestInit | undefined) => Response | Promise<Response>
  >,
) {
  mockedApiFetch.mockImplementation(async (input, init) => {
    const url = String(input);
    for (const [pattern, handler] of Object.entries(routes)) {
      const method = init?.method ?? "GET";
      const [patternUrl, patternMethod] = pattern.split(" ");
      if (
        (patternMethod ? patternMethod === method : method === "GET") &&
        (patternUrl === url || url.startsWith(patternUrl))
      ) {
        return handler(init);
      }
    }
    throw new Error(`Unexpected API request: ${url} ${init?.method ?? "GET"}`);
  });
}

const CREATED_VAULT = {
  vault_id: "00000000-0000-4000-8000-000000000099",
  name: "Field notes",
  enabled: true,
  source: { type: "local", path: "/notes" },
  exclude_patterns: [],
  credential_configured: false,
  activation: "active",
  local_content: "read_write",
  search: "ready",
  git: "disabled",
  watcher: "running",
  capabilities: {
    browse: true,
    search: true,
    mutate: true,
    pull: false,
    push: false,
    retry: false,
  },
};

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("VaultCreationDialog — opening", () => {
  it("renders the name field, source-kind toggle and the four own-folder behaviours", () => {
    render(<VaultCreationDialog onClose={() => {}} onCreated={() => {}} />);

    expect(screen.getByRole("dialog", { name: "Add a Vault" })).toBeVisible();
    expect(screen.getByLabelText("Vault name")).toBeVisible();
    expect(screen.getByLabelText("Folder path")).toBeVisible();
    for (const label of ["No Git", "Local history", "Pull-only", "Two-way"]) {
      expect(screen.getByRole("button", { name: label })).toBeVisible();
    }
  });
});

describe("VaultCreationDialog — a successful local-Vault create", () => {
  it("submits the plain local source shape and hands the created Vault back", async () => {
    let postedBody: Record<string, unknown> | null = null;
    mockRoutes({
      "/api/v1/vaults GET": () =>
        json({
          registry_revision: 5,
          collection_revision: 5,
          vaults: [],
          demo_mode: false,
        }),
      "/api/v1/vaults POST": (init) => {
        postedBody = JSON.parse(init!.body as string);
        return json(
          {
            vault: CREATED_VAULT,
            registry_revision: 6,
            collection_revision: 6,
          },
          { status: 201 },
        );
      },
    });
    const onCreated = vi.fn();

    render(<VaultCreationDialog onClose={() => {}} onCreated={onCreated} />);

    fireEvent.change(screen.getByLabelText("Vault name"), {
      target: { value: "Field notes" },
    });
    fireEvent.change(screen.getByLabelText("Folder path"), {
      target: { value: "/notes" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Create Vault" }));

    await vi.waitFor(() =>
      expect(onCreated).toHaveBeenCalledWith(CREATED_VAULT),
    );
    expect(postedBody).toEqual({
      expected_registry_revision: 5,
      name: "Field notes",
      source: { type: "local", path: "/notes" },
    });
  });

  it("trims a pasted folder path before sending it, same as every other field", async () => {
    let postedBody: Record<string, unknown> | null = null;
    mockRoutes({
      "/api/v1/vaults GET": () =>
        json({
          registry_revision: 5,
          collection_revision: 5,
          vaults: [],
          demo_mode: false,
        }),
      "/api/v1/vaults POST": (init) => {
        postedBody = JSON.parse(init!.body as string);
        return json(
          {
            vault: CREATED_VAULT,
            registry_revision: 6,
            collection_revision: 6,
          },
          { status: 201 },
        );
      },
    });

    render(<VaultCreationDialog onClose={() => {}} onCreated={() => {}} />);

    fireEvent.change(screen.getByLabelText("Vault name"), {
      target: { value: "Field notes" },
    });
    fireEvent.change(screen.getByLabelText("Folder path"), {
      target: { value: "  /notes  \n" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Create Vault" }));

    await vi.waitFor(() => expect(postedBody).not.toBeNull());
    expect(
      (postedBody as unknown as { source: { path: string } }).source.path,
    ).toBe("/notes");
  });
});

describe("VaultCreationDialog — a managed Git create", () => {
  it("submits a managed_git source with the chosen repository URL", async () => {
    let postedBody: Record<string, unknown> | null = null;
    mockRoutes({
      "/api/v1/vaults GET": () =>
        json({
          registry_revision: 1,
          collection_revision: 1,
          vaults: [],
          demo_mode: false,
        }),
      "/api/v1/vaults POST": (init) => {
        postedBody = JSON.parse(init!.body as string);
        return json(
          {
            vault: CREATED_VAULT,
            registry_revision: 2,
            collection_revision: 2,
          },
          { status: 201 },
        );
      },
    });

    render(<VaultCreationDialog onClose={() => {}} onCreated={() => {}} />);

    fireEvent.change(screen.getByLabelText("Vault name"), {
      target: { value: "Clone" },
    });
    fireEvent.click(
      screen.getByRole("button", { name: "A managed Git checkout" }),
    );
    fireEvent.change(screen.getByLabelText("Repository URL"), {
      target: { value: "https://example.test/repo.git" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Create Vault" }));

    await vi.waitFor(() => expect(postedBody).not.toBeNull());
    expect(postedBody).toMatchObject({
      name: "Clone",
      source: {
        type: "managed_git",
        repository_url: "https://example.test/repo.git",
        mode: "pull_only",
      },
    });
  });
});

describe("VaultCreationDialog — an API failure", () => {
  it("shows the server's message and keeps the entered fields", async () => {
    mockRoutes({
      "/api/v1/vaults GET": () =>
        json({
          registry_revision: 5,
          collection_revision: 5,
          vaults: [],
          demo_mode: false,
        }),
      "/api/v1/vaults POST": () =>
        json(
          {
            code: "duplicate_vault_name",
            message: "A Vault named this already exists.",
          },
          { status: 409 },
        ),
    });
    const onCreated = vi.fn();
    const onClose = vi.fn();

    render(<VaultCreationDialog onClose={onClose} onCreated={onCreated} />);

    fireEvent.change(screen.getByLabelText("Vault name"), {
      target: { value: "Field notes" },
    });
    fireEvent.change(screen.getByLabelText("Folder path"), {
      target: { value: "/notes" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Create Vault" }));

    expect(
      await screen.findByText("A Vault named this already exists."),
    ).toBeVisible();
    expect(onCreated).not.toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();
    expect(screen.getByLabelText("Vault name")).toHaveValue("Field notes");
    expect(screen.getByLabelText("Folder path")).toHaveValue("/notes");
  });

  it("gives a plain-language message for a registry revision conflict", async () => {
    mockRoutes({
      "/api/v1/vaults GET": () =>
        json({
          registry_revision: 5,
          collection_revision: 5,
          vaults: [],
          demo_mode: false,
        }),
      "/api/v1/vaults POST": () =>
        json(
          {
            code: "registry_revision_conflict",
            message: "expected registry revision 5, current revision is 6",
          },
          { status: 409 },
        ),
    });

    render(<VaultCreationDialog onClose={() => {}} onCreated={() => {}} />);

    fireEvent.change(screen.getByLabelText("Vault name"), {
      target: { value: "Field notes" },
    });
    fireEvent.change(screen.getByLabelText("Folder path"), {
      target: { value: "/notes" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Create Vault" }));

    expect(
      await screen.findByText("This changed elsewhere just now. Try again."),
    ).toBeVisible();
  });
});

describe("VaultCreationDialog — double-submit", () => {
  it("sends only one create request when Create Vault is clicked twice rapidly", async () => {
    let postCount = 0;
    mockRoutes({
      "/api/v1/vaults GET": () =>
        json({
          registry_revision: 5,
          collection_revision: 5,
          vaults: [],
          demo_mode: false,
        }),
      "/api/v1/vaults POST": () => {
        postCount += 1;
        return json(
          {
            vault: CREATED_VAULT,
            registry_revision: 6,
            collection_revision: 6,
          },
          { status: 201 },
        );
      },
    });
    const onCreated = vi.fn();

    render(<VaultCreationDialog onClose={() => {}} onCreated={onCreated} />);

    fireEvent.change(screen.getByLabelText("Vault name"), {
      target: { value: "Field notes" },
    });
    fireEvent.change(screen.getByLabelText("Folder path"), {
      target: { value: "/notes" },
    });
    const button = screen.getByRole("button", { name: "Create Vault" });
    fireEvent.click(button);
    fireEvent.click(button);

    await vi.waitFor(() => expect(onCreated).toHaveBeenCalledTimes(1));
    expect(postCount).toBe(1);
  });
});

describe("VaultCreationDialog — sign-in validation", () => {
  it("requires an access token once Access token sign-in is chosen, without contacting the server", async () => {
    mockRoutes({
      "/api/v1/vaults GET": () =>
        json({
          registry_revision: 5,
          collection_revision: 5,
          vaults: [],
          demo_mode: false,
        }),
    });

    render(<VaultCreationDialog onClose={() => {}} onCreated={() => {}} />);

    fireEvent.change(screen.getByLabelText("Vault name"), {
      target: { value: "Clone" },
    });
    fireEvent.click(
      screen.getByRole("button", { name: "A managed Git checkout" }),
    );
    fireEvent.change(screen.getByLabelText("Repository URL"), {
      target: { value: "https://example.test/repo.git" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Access token" }));
    fireEvent.click(screen.getByRole("button", { name: "Create Vault" }));

    expect(
      await screen.findByText("Enter an access token, or choose No sign-in."),
    ).toBeVisible();
    expect(mockedApiFetch).not.toHaveBeenCalledWith(
      "/api/v1/vaults",
      expect.objectContaining({ method: "POST" }),
    );
  });
});

describe("VaultCreationDialog — an own-folder remote behaviour", () => {
  it("requires a repository URL for Pull-only, same as the edit flow", async () => {
    mockRoutes({
      "/api/v1/vaults GET": () =>
        json({
          registry_revision: 5,
          collection_revision: 5,
          vaults: [],
          demo_mode: false,
        }),
    });

    render(<VaultCreationDialog onClose={() => {}} onCreated={() => {}} />);

    fireEvent.change(screen.getByLabelText("Vault name"), {
      target: { value: "Field notes" },
    });
    fireEvent.change(screen.getByLabelText("Folder path"), {
      target: { value: "/notes" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Pull-only" }));
    fireEvent.click(screen.getByRole("button", { name: "Create Vault" }));

    expect(
      await screen.findByText("A repository is required for this behaviour."),
    ).toBeVisible();
    expect(mockedApiFetch).not.toHaveBeenCalledWith(
      "/api/v1/vaults",
      expect.objectContaining({ method: "POST" }),
    );
  });

  it("drops a token entered for a remote behaviour once switched back to No Git", async () => {
    let postedBody: Record<string, unknown> | null = null;
    mockRoutes({
      "/api/v1/vaults GET": () =>
        json({
          registry_revision: 5,
          collection_revision: 5,
          vaults: [],
          demo_mode: false,
        }),
      "/api/v1/vaults POST": (init) => {
        postedBody = JSON.parse(init!.body as string);
        return json(
          {
            vault: CREATED_VAULT,
            registry_revision: 6,
            collection_revision: 6,
          },
          { status: 201 },
        );
      },
    });

    render(<VaultCreationDialog onClose={() => {}} onCreated={() => {}} />);

    fireEvent.change(screen.getByLabelText("Vault name"), {
      target: { value: "Field notes" },
    });
    fireEvent.change(screen.getByLabelText("Folder path"), {
      target: { value: "/notes" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Pull-only" }));
    fireEvent.change(screen.getByLabelText("Repository URL"), {
      target: { value: "https://example.test/repo.git" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Access token" }));
    fireEvent.change(screen.getByLabelText("Repository access token"), {
      target: { value: "secret-token" },
    });

    fireEvent.click(screen.getByRole("button", { name: "No Git" }));
    // Switching away hides the sign-in control entirely — nothing left on
    // screen to prove the token was cleared except the request that follows.
    expect(
      screen.queryByLabelText("Repository access token"),
    ).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Create Vault" }));

    await vi.waitFor(() => expect(postedBody).not.toBeNull());
    expect(postedBody).toEqual({
      expected_registry_revision: 5,
      name: "Field notes",
      source: { type: "local", path: "/notes" },
    });
  });

  it("does not carry a repository URL typed for an abandoned choice back into a later one", async () => {
    render(<VaultCreationDialog onClose={() => {}} onCreated={() => {}} />);

    fireEvent.click(
      screen.getByRole("button", { name: "A managed Git checkout" }),
    );
    fireEvent.change(screen.getByLabelText("Repository URL"), {
      target: { value: "https://example.test/abandoned.git" },
    });

    fireEvent.click(
      screen.getByRole("button", { name: "A folder on this server" }),
    );
    expect(screen.queryByLabelText("Repository URL")).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Pull-only" }));
    expect(screen.getByLabelText("Repository URL")).toHaveValue("");
  });
});
