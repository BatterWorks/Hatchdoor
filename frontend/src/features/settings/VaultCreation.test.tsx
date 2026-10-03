import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { apiFetch } from "../../api/api";
import { HelpContext } from "../help/useHelp";
import { VaultCreationDialog } from "./VaultCreation";

vi.mock("../../api/api", () => ({ apiFetch: vi.fn() }));
const mockedApiFetch = vi.mocked(apiFetch);

const json = (body: unknown, init?: ResponseInit) =>
  new Response(JSON.stringify(body), {
    headers: { "content-type": "application/json" },
    ...init,
  });

/** The Vault mount as `GET /api/v1/folders` lists it (#430). */
const MOUNT_LISTING = {
  root: "/data/vault",
  root_found: true,
  path: "",
  markdown: { count: 12, at_least: false },
  vault: null,
  folders: [
    {
      name: "Journal",
      path: "Journal",
      markdown: { count: 12, at_least: false },
      vault: null,
      has_subfolders: false,
    },
  ],
  skipped_invalid_names: 0,
};

function mockRoutes(
  routes: Record<
    string,
    (init: RequestInit | undefined) => Response | Promise<Response>
  >,
) {
  // The folder picker lists the mount as soon as the dialog opens.
  const withFolders: typeof routes = {
    "/api/v1/folders": () => json(MOUNT_LISTING),
    ...routes,
  };
  mockedApiFetch.mockImplementation(async (input, init) => {
    const url = String(input);
    for (const [pattern, handler] of Object.entries(withFolders)) {
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

/** The typed Folder path field, behind "Type a path instead" since the
 * folder picker became the default (#430). */
function typedPathField(): HTMLElement {
  const toggle = screen.queryByRole("button", { name: "Type a path instead" });
  if (toggle) fireEvent.click(toggle);
  return screen.getByLabelText("Folder path");
}

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("VaultCreationDialog — opening", () => {
  it("renders the name field, source-kind toggle and the four own-folder behaviours", async () => {
    mockRoutes({});
    render(<VaultCreationDialog onClose={() => {}} onCreated={() => {}} />);

    expect(screen.getByRole("dialog", { name: "Add a Vault" })).toBeVisible();
    expect(screen.getByLabelText("Vault name")).toBeVisible();
    expect(
      screen.getByLabelText("Ignore these files and folders"),
    ).toBeVisible();
    expect(
      await screen.findByRole("button", { name: /^Journal/ }),
    ).toBeVisible();
    expect(typedPathField()).toBeVisible();
    for (const label of ["No Git", "Local history", "Pull-only", "Two-way"]) {
      expect(screen.getByRole("button", { name: label })).toBeVisible();
    }
  });
});

describe("VaultCreationDialog — exclusion patterns", () => {
  it("sends normalized exclude_patterns from the initial create request", async () => {
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
    fireEvent.change(typedPathField(), {
      target: { value: "/notes" },
    });
    fireEvent.change(screen.getByLabelText("Ignore these files and folders"), {
      target: { value: " node_modules , .git ,, dist " },
    });
    fireEvent.click(screen.getByRole("button", { name: "Create Vault" }));

    await vi.waitFor(() => expect(postedBody).not.toBeNull());
    expect(postedBody).toEqual({
      expected_registry_revision: 5,
      name: "Field notes",
      source: { type: "local", path: "/notes" },
      exclude_patterns: ["node_modules", ".git", "dist"],
    });
  });

  it("omits exclude_patterns entirely when the field is left empty", async () => {
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
    fireEvent.change(typedPathField(), {
      target: { value: "/notes" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Create Vault" }));

    await vi.waitFor(() => expect(postedBody).not.toBeNull());
    expect(postedBody).not.toHaveProperty("exclude_patterns");
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
    fireEvent.change(typedPathField(), {
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
    fireEvent.change(typedPathField(), {
      target: { value: "  /notes  \n" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Create Vault" }));

    await vi.waitFor(() => expect(postedBody).not.toBeNull());
    expect(
      (postedBody as unknown as { source: { path: string } }).source.path,
    ).toBe("/notes");
  });
});

describe("VaultCreationDialog — a folder picked from the list (#430)", () => {
  const registry = () =>
    json({
      registry_revision: 5,
      collection_revision: 5,
      vaults: [],
      demo_mode: false,
    });

  it("creates a Vault from a picked folder without typing a path", async () => {
    let postedBody: Record<string, unknown> | null = null;
    mockRoutes({
      "/api/v1/vaults GET": registry,
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
    fireEvent.click(await screen.findByRole("button", { name: /^Journal/ }));
    expect(screen.getByText("/data/vault/Journal")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Create Vault" }));

    await vi.waitFor(() =>
      expect(onCreated).toHaveBeenCalledWith(CREATED_VAULT),
    );
    expect(postedBody).toEqual({
      expected_registry_revision: 5,
      name: "Field notes",
      source: { type: "local", path: "/data/vault/Journal" },
    });
    expect(screen.queryByLabelText("Folder path")).toBeNull();
  });

  it("asks for a folder when none was picked, without contacting the server", async () => {
    mockRoutes({});
    render(<VaultCreationDialog onClose={() => {}} onCreated={() => {}} />);

    fireEvent.change(screen.getByLabelText("Vault name"), {
      target: { value: "Field notes" },
    });
    await screen.findByRole("button", { name: /^Journal/ });
    fireEvent.click(screen.getByRole("button", { name: "Create Vault" }));

    expect(screen.getByRole("alert")).toHaveTextContent(
      "Pick the folder that holds your notes, or type its path.",
    );
    expect(mockedApiFetch).toHaveBeenCalledTimes(1);
  });

  it("carries a picked folder into the typed field and back", async () => {
    mockRoutes({});
    render(<VaultCreationDialog onClose={() => {}} onCreated={() => {}} />);

    fireEvent.click(await screen.findByRole("button", { name: /^Journal/ }));
    expect(typedPathField()).toHaveValue("/data/vault/Journal");

    fireEvent.click(
      screen.getByRole("button", { name: "Pick from the list instead" }),
    );
    expect(
      await screen.findByRole("button", { name: /^Journal/ }),
    ).toHaveAttribute("aria-pressed", "true");
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
    fireEvent.change(typedPathField(), {
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
    fireEvent.change(typedPathField(), {
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
    fireEvent.change(typedPathField(), {
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
    fireEvent.change(typedPathField(), {
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
    fireEvent.change(typedPathField(), {
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

describe("VaultCreationDialog — keyboard (#338)", () => {
  it("starts on the name field, keeps Tab inside, and closes on Escape", () => {
    const onClose = vi.fn();
    render(<VaultCreationDialog onClose={onClose} onCreated={() => {}} />);

    const dialog = screen.getByRole("dialog", { name: "Add a Vault" });
    const name = screen.getByLabelText("Vault name");
    expect(name).toHaveFocus();

    const create = screen.getByRole("button", { name: "Create Vault" });
    create.focus();
    fireEvent.keyDown(create, { key: "Tab" });
    expect(name).toHaveFocus();
    fireEvent.keyDown(name, { key: "Tab", shiftKey: true });
    expect(create).toHaveFocus();
    expect(dialog).toContainElement(document.activeElement as HTMLElement);

    fireEvent.keyDown(create, { key: "Escape" });
    expect(onClose).toHaveBeenCalledTimes(1);
  });
  it("closes Help first while it is open beside the dialog, and lets Tab reach it (#430)", () => {
    mockRoutes({});
    const onClose = vi.fn();
    const closeHelp = vi.fn();
    render(
      <HelpContext.Provider
        value={{ openHelp: () => {}, closeHelp, isOpen: true }}
      >
        <VaultCreationDialog onClose={onClose} onCreated={() => {}} />
      </HelpContext.Provider>,
    );

    const create = screen.getByRole("button", { name: "Create Vault" });
    create.focus();
    fireEvent.keyDown(create, { key: "Tab" });
    expect(screen.getByLabelText("Vault name")).not.toHaveFocus();

    fireEvent.keyDown(create, { key: "Escape" });
    expect(closeHelp).toHaveBeenCalledTimes(1);
    expect(onClose).not.toHaveBeenCalled();
    expect(
      screen.getByRole("dialog", { name: "Add a Vault" }).parentElement,
    ).toHaveClass("is-beside-help");
  });
});
