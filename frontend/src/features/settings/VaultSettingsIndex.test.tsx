import {
  cleanup,
  fireEvent,
  render,
  screen,
  within,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { apiFetch } from "../../api/api";
import { VaultSettingsDetail, VaultSettingsIndex } from "./VaultSettingsIndex";
import {
  behaviorOptions,
  buildSourceForBehavior,
  clampPollMinutes,
  describeGitFailure,
  isRecoveryPending,
  markRecoveryPending,
  sameSourceIdentity,
  withIdentityFields,
} from "./vaultGitBehavior";

vi.mock("../../api/api", () => ({
  apiFetch: vi.fn(),
  // The Vault collection client opens the shared revision stream through this.
  withAccessToken: (url: string) => url,
}));
const mockedApiFetch = vi.mocked(apiFetch);

const VAULT_ID = "00000000-0000-4000-8000-000000000001";
const SERVER_IDENTITY = { name: "Server author", email: "author@example.test" };

const json = (body: unknown) =>
  new Response(JSON.stringify(body), {
    headers: { "content-type": "application/json" },
  });

function baseVault(source: unknown, overrides: Record<string, unknown> = {}) {
  return {
    vault_id: VAULT_ID,
    name: "Field notes",
    enabled: true,
    source,
    exclude_patterns: [],
    credential_configured: false,
    archive_folder: undefined,
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
    ...overrides,
  };
}

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

function mockDetail(
  vault: unknown,
  {
    onPatch,
    extraRoutes = {},
  }: {
    onPatch?: (body: Record<string, unknown>) => void;
    extraRoutes?: Record<
      string,
      (init: RequestInit | undefined) => Response | Promise<Response>
    >;
  } = {},
) {
  mockRoutes({
    "/api/v1/vaults": () =>
      json({
        registry_revision: 3,
        collection_revision: 3,
        vaults: [vault],
        demo_mode: false,
      }),
    "/api/v1/vaults/all/stats": () =>
      json({ data: [{ vault_id: VAULT_ID, note_count: 12 }] }),
    [`/api/v1/vaults/${VAULT_ID}/recent?limit=1`]: () =>
      json({ data: [{ mtime_ns: 0 }] }),
    [`/api/v1/vaults/${VAULT_ID} PATCH`]: (init) => {
      const body = JSON.parse(String(init?.body)) as Record<string, unknown>;
      onPatch?.(body);
      return json({
        vault: { ...(vault as object), ...body },
        registry_revision: 4,
        collection_revision: 4,
      });
    },
    ...extraRoutes,
  });
}

beforeEach(() => {
  localStorage.clear();
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.clearAllMocks();
  localStorage.clear();
});

describe("pure helpers", () => {
  it("offers four behaviours on an owned folder, two on a cloned one", () => {
    expect(
      behaviorOptions({ type: "local", path: "/notes" }).map((item) => item.id),
    ).toEqual(["no_git", "local_history", "pull_only", "two_way"]);
    expect(
      behaviorOptions({
        type: "existing_git",
        repository_path: "/notes",
        mode: "pull_only",
        poll_interval_secs: 60,
      }).map((item) => item.id),
    ).toEqual(["no_git", "local_history", "pull_only", "two_way"]);
    expect(
      behaviorOptions({
        type: "managed_git",
        repository_url: "https://example.test/notes.git",
        mode: "two_way",
        poll_interval_secs: 60,
      }).map((item) => item.id),
    ).toEqual(["pull_only", "two_way"]);
  });

  it("carries the folder's own path across when turning Git on or off", () => {
    const local = { type: "local" as const, path: "/notes" };
    const asGit = buildSourceForBehavior(local, "pull_only");
    expect(asGit).toMatchObject({
      type: "existing_git",
      repository_path: "/notes",
      mode: "pull_only",
    });
    const backToLocal = buildSourceForBehavior(asGit, "no_git");
    expect(backToLocal).toEqual({ type: "local", path: "/notes" });
  });

  it("swaps only mode between the three Git behaviours on an owned folder", () => {
    const source = {
      type: "existing_git" as const,
      repository_path: "/notes",
      repository_url: "https://example.test/notes.git",
      branch: "main",
      mode: "pull_only" as const,
      poll_interval_secs: 120,
    };
    const next = buildSourceForBehavior(source, "two_way");
    expect(next).toEqual({ ...source, mode: "two_way" });
  });

  it("refuses local_history when swapping a cloned folder's behaviour", () => {
    const source = {
      type: "managed_git" as const,
      repository_url: "https://example.test/notes.git",
      mode: "pull_only" as const,
      poll_interval_secs: 120,
    };
    expect(buildSourceForBehavior(source, "local_history")).toEqual(source);
  });

  it("treats mode and poll interval as outside identity, everything else as identity", () => {
    const a = {
      type: "existing_git" as const,
      repository_path: "/notes",
      repository_url: "https://example.test/notes.git",
      branch: "main",
      mode: "pull_only" as const,
      poll_interval_secs: 60,
    };
    expect(
      sameSourceIdentity(a, { ...a, mode: "two_way", poll_interval_secs: 999 }),
    ).toBe(true);
    expect(sameSourceIdentity(a, { ...a, branch: "dev" })).toBe(false);
    expect(
      sameSourceIdentity(a, { type: "local", path: a.repository_path }),
    ).toBe(false);
  });

  it("never edits repository_path — only repository, branch and folder", () => {
    const base = {
      type: "existing_git" as const,
      repository_path: "/notes",
      mode: "pull_only" as const,
      poll_interval_secs: 60,
    };
    const edited = withIdentityFields(base, {
      repositoryUrl: "https://example.test/new.git",
      branch: "dev",
      subdirectory: "vault",
      pollMinutes: 30,
    });
    expect(edited).toEqual({
      type: "existing_git",
      repository_path: "/notes",
      mode: "pull_only",
      repository_url: "https://example.test/new.git",
      branch: "dev",
      vault_subdirectory: "vault",
      poll_interval_secs: 1800,
    });
  });

  it("clamps the schedule to 1..1440 minutes, defaulting to 1440", () => {
    expect(clampPollMinutes("1440")).toBe(1440);
    expect(clampPollMinutes("0")).toBe(1);
    expect(clampPollMinutes("5000")).toBe(1440);
    expect(clampPollMinutes("not a number")).toBe(1440);
  });

  it("describes each of the nine failures with a sentence, a tier and file lists where data carries them", () => {
    const withoutFiles = describeGitFailure({
      code: "managed_git_authentication_failed",
      message: "auth failed",
      retryable: false,
    });
    expect(withoutFiles.tier).toBe("error");
    expect(withoutFiles.files).toBeUndefined();

    const withFiles = describeGitFailure({
      code: "managed_git_conflict",
      message: "conflict",
      retryable: false,
      detail: { kind: "affected_paths", paths: ["a.md", "b.md"], total: 2 },
    });
    expect(withFiles.files).toEqual(["a.md", "b.md"]);

    const unknown = describeGitFailure({
      code: "managed_git_not_remote",
      message: "not a remote vault",
      retryable: false,
    });
    expect(unknown.sentence).toContain("not a remote vault");
  });

  it("persists a recovery marker across a reload", () => {
    expect(isRecoveryPending(VAULT_ID)).toBe(false);
    markRecoveryPending(VAULT_ID);
    expect(isRecoveryPending(VAULT_ID)).toBe(true);
  });
});

describe("VaultSettingsDetail — the Git behaviour control", () => {
  it("shows no Git behaviour control for a Local Vault, and offers all four options once one exists", async () => {
    mockDetail(baseVault({ type: "local", path: "/notes" }));
    render(
      <VaultSettingsDetail
        vaultId={VAULT_ID}
        serverIdentity={SERVER_IDENTITY}
        onDisconnect={() => {}}
      />,
    );

    await screen.findByRole("heading", { name: "Field notes" });
    const group = screen.getByRole("group", { name: "Git behaviour" });
    expect(
      within(group).getByRole("button", { name: "No Git" }),
    ).toHaveAttribute("aria-pressed", "true");
    expect(
      within(group).getByRole("button", { name: "Local history" }),
    ).toBeVisible();
    expect(
      within(group).getByRole("button", { name: "Pull-only" }),
    ).toBeVisible();
    expect(
      within(group).getByRole("button", { name: "Two-way" }),
    ).toBeVisible();
  });

  it("offers only Pull-only and Two-way for a managed Git checkout", async () => {
    mockDetail(
      baseVault({
        type: "managed_git",
        repository_url: "https://example.test/notes.git",
        branch: "main",
        vault_subdirectory: null,
        mode: "two_way",
        poll_interval_secs: 3600,
      }),
    );
    render(
      <VaultSettingsDetail
        vaultId={VAULT_ID}
        serverIdentity={SERVER_IDENTITY}
        onDisconnect={() => {}}
      />,
    );

    await screen.findByRole("heading", { name: "Field notes" });
    const group = screen.getByRole("group", { name: "Git behaviour" });
    expect(
      within(group).queryByRole("button", { name: "No Git" }),
    ).not.toBeInTheDocument();
    expect(
      within(group).queryByRole("button", { name: "Local history" }),
    ).not.toBeInTheDocument();
    expect(
      within(group).getByRole("button", { name: "Two-way" }),
    ).toHaveAttribute("aria-pressed", "true");
  });

  it("saves a plain mode swap on an owned folder as a single PATCH, no confirmation", async () => {
    let patched: Record<string, unknown> | undefined;
    mockDetail(
      baseVault({
        type: "existing_git",
        repository_path: "/notes",
        repository_url: "https://example.test/notes.git",
        branch: "main",
        vault_subdirectory: null,
        mode: "pull_only",
        poll_interval_secs: 3600,
      }),
      { onPatch: (body) => (patched = body) },
    );
    render(
      <VaultSettingsDetail
        vaultId={VAULT_ID}
        serverIdentity={SERVER_IDENTITY}
        onDisconnect={() => {}}
      />,
    );

    await screen.findByRole("heading", { name: "Field notes" });
    fireEvent.click(screen.getByRole("button", { name: "Two-way" }));
    fireEvent.click(screen.getByRole("button", { name: "Save Vault" }));

    await screen.findByText("Saved.");
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    const source = patched?.source as Record<string, unknown>;
    expect(source.mode).toBe("two_way");
    expect(source.repository_url).toBe("https://example.test/notes.git");
  });

  it("requires a refuse-then-confirm round trip to turn Git on, running pause, edit and un-pause as one act", async () => {
    let patchBody: Record<string, unknown> | undefined;
    let sawDisableFirst = false;
    let sawEnableLast = false;
    const calls: string[] = [];
    mockDetail(baseVault({ type: "local", path: "/notes" }), {
      extraRoutes: {
        [`/api/v1/vaults/${VAULT_ID}/disable POST`]: () => {
          calls.push("disable");
          return json({ registry_revision: 4, collection_revision: 4 });
        },
        [`/api/v1/vaults/${VAULT_ID} PATCH`]: (init) => {
          calls.push("edit");
          patchBody = JSON.parse(String(init?.body)) as Record<string, unknown>;
          sawDisableFirst = calls[0] === "disable";
          return json({
            vault: baseVault(patchBody.source, { enabled: false }),
            registry_revision: 5,
            collection_revision: 5,
          });
        },
        [`/api/v1/vaults/${VAULT_ID}/enable POST`]: () => {
          calls.push("enable");
          sawEnableLast = calls[calls.length - 1] === "enable";
          return json({
            vault: baseVault(patchBody?.source, { enabled: true }),
            registry_revision: 6,
            collection_revision: 6,
          });
        },
      },
    });
    render(
      <VaultSettingsDetail
        vaultId={VAULT_ID}
        serverIdentity={SERVER_IDENTITY}
        onDisconnect={() => {}}
      />,
    );

    await screen.findByRole("heading", { name: "Field notes" });
    fireEvent.click(screen.getByRole("button", { name: "Local history" }));
    fireEvent.click(screen.getByRole("button", { name: "Save Vault" }));

    const dialog = await screen.findByRole("dialog", {
      name: "Before this is saved",
    });
    expect(dialog).toHaveTextContent(/hidden \.git folder/i);
    expect(dialog).toHaveTextContent(/grows permanently/i);
    fireEvent.click(within(dialog).getByRole("button", { name: "Go ahead" }));

    await screen.findByText("Saved.");
    expect(calls).toEqual(["disable", "edit", "enable"]);
    expect(sawDisableFirst).toBe(true);
    expect(sawEnableLast).toBe(true);
    expect((patchBody?.source as Record<string, unknown>).type).toBe(
      "existing_git",
    );
    expect(patchBody?.confirm_identity_change).toBe(true);
  });

  it("restores the Vault and reports nothing changed when the edit step fails", async () => {
    mockDetail(baseVault({ type: "local", path: "/notes" }), {
      extraRoutes: {
        [`/api/v1/vaults/${VAULT_ID}/disable POST`]: () =>
          json({ registry_revision: 4, collection_revision: 4 }),
        [`/api/v1/vaults/${VAULT_ID} PATCH`]: () =>
          new Response(
            JSON.stringify({
              message: "Something about this edit was refused.",
            }),
            { status: 409, headers: { "content-type": "application/json" } },
          ),
        [`/api/v1/vaults/${VAULT_ID}/enable POST`]: () =>
          json({
            vault: baseVault(
              { type: "local", path: "/notes" },
              { enabled: true },
            ),
            registry_revision: 5,
            collection_revision: 5,
          }),
      },
    });
    render(
      <VaultSettingsDetail
        vaultId={VAULT_ID}
        serverIdentity={SERVER_IDENTITY}
        onDisconnect={() => {}}
      />,
    );

    await screen.findByRole("heading", { name: "Field notes" });
    fireEvent.click(screen.getByRole("button", { name: "Local history" }));
    fireEvent.click(screen.getByRole("button", { name: "Save Vault" }));
    fireEvent.click(await screen.findByRole("button", { name: "Go ahead" }));

    await screen.findByText(/nothing changed/i);
    expect(isRecoveryPending(VAULT_ID)).toBe(false);
  });

  it("also reports nothing changed, with the server's own reason, when the pause step itself fails", async () => {
    mockDetail(baseVault({ type: "local", path: "/notes" }), {
      extraRoutes: {
        [`/api/v1/vaults/${VAULT_ID}/disable POST`]: () =>
          new Response(
            JSON.stringify({ message: "This Vault is busy right now." }),
            { status: 409, headers: { "content-type": "application/json" } },
          ),
      },
    });
    render(
      <VaultSettingsDetail
        vaultId={VAULT_ID}
        serverIdentity={SERVER_IDENTITY}
        onDisconnect={() => {}}
      />,
    );

    await screen.findByRole("heading", { name: "Field notes" });
    fireEvent.click(screen.getByRole("button", { name: "Local history" }));
    fireEvent.click(screen.getByRole("button", { name: "Save Vault" }));
    fireEvent.click(await screen.findByRole("button", { name: "Go ahead" }));

    await screen.findByText(/nothing changed.*this vault is busy right now/i);
  });

  it("rolls back and reports nothing changed, rather than hanging forever, when the edit step's request itself fails (network error, not just a bad response)", async () => {
    mockDetail(baseVault({ type: "local", path: "/notes" }), {
      extraRoutes: {
        [`/api/v1/vaults/${VAULT_ID}/disable POST`]: () =>
          json({ registry_revision: 4, collection_revision: 4 }),
        [`/api/v1/vaults/${VAULT_ID} PATCH`]: () => {
          throw new TypeError("Failed to fetch");
        },
        [`/api/v1/vaults/${VAULT_ID}/enable POST`]: () =>
          json({
            vault: baseVault(
              { type: "local", path: "/notes" },
              { enabled: true },
            ),
            registry_revision: 5,
            collection_revision: 5,
          }),
      },
    });
    render(
      <VaultSettingsDetail
        vaultId={VAULT_ID}
        serverIdentity={SERVER_IDENTITY}
        onDisconnect={() => {}}
      />,
    );

    await screen.findByRole("heading", { name: "Field notes" });
    fireEvent.click(screen.getByRole("button", { name: "Local history" }));
    fireEvent.click(screen.getByRole("button", { name: "Save Vault" }));
    fireEvent.click(await screen.findByRole("button", { name: "Go ahead" }));

    await screen.findByText(/nothing changed/i);
    expect(screen.getByRole("button", { name: "Save Vault" })).toBeEnabled();
    expect(isRecoveryPending(VAULT_ID)).toBe(false);
  });

  it("leaves a persistent red-line recovery state when the final un-pause fails", async () => {
    mockDetail(baseVault({ type: "local", path: "/notes" }), {
      extraRoutes: {
        [`/api/v1/vaults/${VAULT_ID}/disable POST`]: () =>
          json({ registry_revision: 4, collection_revision: 4 }),
        [`/api/v1/vaults/${VAULT_ID} PATCH`]: (init) => {
          const body = JSON.parse(String(init?.body)) as Record<
            string,
            unknown
          >;
          return json({
            vault: baseVault(body.source, { enabled: false }),
            registry_revision: 5,
            collection_revision: 5,
          });
        },
        [`/api/v1/vaults/${VAULT_ID}/enable POST`]: () =>
          new Response(JSON.stringify({ message: "could not restart" }), {
            status: 500,
            headers: { "content-type": "application/json" },
          }),
      },
    });
    render(
      <VaultSettingsDetail
        vaultId={VAULT_ID}
        serverIdentity={SERVER_IDENTITY}
        onDisconnect={() => {}}
      />,
    );

    await screen.findByRole("heading", { name: "Field notes" });
    fireEvent.click(screen.getByRole("button", { name: "Local history" }));
    fireEvent.click(screen.getByRole("button", { name: "Save Vault" }));
    fireEvent.click(await screen.findByRole("button", { name: "Go ahead" }));

    await screen.findByRole("alert");
    expect(isRecoveryPending(VAULT_ID)).toBe(true);
    expect(
      screen.getByRole("button", { name: "Try to bring this Vault back" }),
    ).toBeVisible();
  });
});

describe("VaultSettingsDetail — sign-in", () => {
  it("shows no sign-in control on a Local Vault or an owned folder kept at Local history", async () => {
    mockDetail(baseVault({ type: "local", path: "/notes" }));
    render(
      <VaultSettingsDetail
        vaultId={VAULT_ID}
        serverIdentity={SERVER_IDENTITY}
        onDisconnect={() => {}}
      />,
    );
    await screen.findByRole("heading", { name: "Field notes" });
    expect(
      screen.queryByRole("group", { name: "Sign-in" }),
    ).not.toBeInTheDocument();
  });

  it("has one control with no separate Remove — choosing No sign-in is how a token is forgotten", async () => {
    let patched: Record<string, unknown> | undefined;
    mockDetail(
      baseVault(
        {
          type: "managed_git",
          repository_url: "https://example.test/notes.git",
          branch: "main",
          mode: "two_way",
          poll_interval_secs: 3600,
        },
        { credential_configured: true },
      ),
      { onPatch: (body) => (patched = body) },
    );
    render(
      <VaultSettingsDetail
        vaultId={VAULT_ID}
        serverIdentity={SERVER_IDENTITY}
        onDisconnect={() => {}}
      />,
    );

    await screen.findByRole("heading", { name: "Field notes" });
    expect(
      screen.queryByRole("button", { name: "Remove" }),
    ).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "No sign-in" }));
    fireEvent.click(screen.getByRole("button", { name: "Save Vault" }));

    await screen.findByText("Saved.");
    expect(patched?.https_credentials).toEqual({ action: "remove" });
  });

  it("shows the token's state as a word beside the label, flipping to a warn-ink 'will be cleared' on an identity change", async () => {
    mockDetail(
      baseVault(
        {
          type: "managed_git",
          repository_url: "https://example.test/notes.git",
          branch: "main",
          mode: "two_way",
          poll_interval_secs: 3600,
        },
        { credential_configured: true },
      ),
    );
    render(
      <VaultSettingsDetail
        vaultId={VAULT_ID}
        serverIdentity={SERVER_IDENTITY}
        onDisconnect={() => {}}
      />,
    );

    await screen.findByRole("heading", { name: "Field notes" });
    const saved = screen.getByText("saved");
    expect(saved).toHaveClass("settings-token-state");
    expect(saved).not.toHaveClass("settings-token-state-warn");

    fireEvent.click(screen.getByRole("button", { name: "Edit" }));
    fireEvent.change(screen.getByLabelText("Branch"), {
      target: { value: "dev" },
    });

    const warned = screen.getByText("will be cleared");
    expect(warned).toHaveClass("settings-token-state-warn");
  });

  it("keeps the token field always empty and sends replace when a new one is typed", async () => {
    let patched: Record<string, unknown> | undefined;
    mockDetail(
      baseVault(
        {
          type: "managed_git",
          repository_url: "https://example.test/notes.git",
          branch: "main",
          mode: "two_way",
          poll_interval_secs: 3600,
        },
        { credential_configured: true },
      ),
      { onPatch: (body) => (patched = body) },
    );
    render(
      <VaultSettingsDetail
        vaultId={VAULT_ID}
        serverIdentity={SERVER_IDENTITY}
        onDisconnect={() => {}}
      />,
    );

    await screen.findByRole("heading", { name: "Field notes" });
    const token = screen.getByLabelText(
      "Repository access token",
    ) as HTMLInputElement;
    expect(token.value).toBe("");
    fireEvent.change(token, { target: { value: "s3cr3t" } });
    fireEvent.click(screen.getByRole("button", { name: "Save Vault" }));

    await screen.findByText("Saved.");
    expect(patched?.https_credentials).toEqual({
      action: "replace",
      token: "s3cr3t",
    });
  });

  it("sends keep when the token field is left blank", async () => {
    let patched: Record<string, unknown> | undefined;
    mockDetail(
      baseVault(
        {
          type: "managed_git",
          repository_url: "https://example.test/notes.git",
          branch: "main",
          mode: "two_way",
          poll_interval_secs: 3600,
        },
        { credential_configured: true },
      ),
      { onPatch: (body) => (patched = body) },
    );
    render(
      <VaultSettingsDetail
        vaultId={VAULT_ID}
        serverIdentity={SERVER_IDENTITY}
        onDisconnect={() => {}}
      />,
    );

    await screen.findByRole("heading", { name: "Field notes" });
    fireEvent.click(screen.getByRole("button", { name: "Save Vault" }));

    await screen.findByText("Saved.");
    expect(patched?.https_credentials).toEqual({ action: "keep" });
  });
});

describe("VaultSettingsDetail — schedule", () => {
  it("shows the schedule field only for a remote-backed behaviour, defaulting to 1440 minutes", async () => {
    mockDetail(baseVault({ type: "local", path: "/notes" }));
    render(
      <VaultSettingsDetail
        vaultId={VAULT_ID}
        serverIdentity={SERVER_IDENTITY}
        onDisconnect={() => {}}
      />,
    );
    await screen.findByRole("heading", { name: "Field notes" });
    expect(
      screen.queryByLabelText("Sync schedule in minutes"),
    ).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Pull-only" }));
    const field = screen.getByLabelText(
      "Sync schedule in minutes",
    ) as HTMLInputElement;
    expect(field.value).toBe("1440");
  });
});

describe("VaultSettingsDetail — sync console", () => {
  it("shows a healthy console with a Sync now button for a working remote Vault", async () => {
    mockDetail(
      baseVault(
        {
          type: "managed_git",
          repository_url: "https://example.test/notes.git",
          branch: "main",
          mode: "two_way",
          poll_interval_secs: 3600,
        },
        { git: "ready" },
      ),
    );
    render(
      <VaultSettingsDetail
        vaultId={VAULT_ID}
        serverIdentity={SERVER_IDENTITY}
        onDisconnect={() => {}}
      />,
    );
    await screen.findByRole("heading", { name: "Field notes" });
    expect(screen.getByText("Healthy")).toBeVisible();
    expect(screen.getByRole("button", { name: "Sync now" })).toBeVisible();
  });

  it("shows the matching sentence, file list and Try again button for a failing Vault", async () => {
    mockDetail(
      baseVault(
        {
          type: "managed_git",
          repository_url: "https://example.test/notes.git",
          branch: "main",
          mode: "two_way",
          poll_interval_secs: 3600,
        },
        {
          git: "unavailable",
          git_error: {
            code: "managed_git_conflict",
            message: "conflict",
            retryable: false,
            detail: {
              kind: "affected_paths",
              paths: ["notes/a.md", "notes/b.md"],
              total: 2,
            },
          },
        },
      ),
    );
    render(
      <VaultSettingsDetail
        vaultId={VAULT_ID}
        serverIdentity={SERVER_IDENTITY}
        onDisconnect={() => {}}
      />,
    );
    await screen.findByRole("heading", { name: "Field notes" });
    expect(
      screen.getByText(/conflict between this Vault and its remote/i),
    ).toBeVisible();
    expect(screen.getByText("notes/a.md")).toBeVisible();
    expect(screen.getByText("notes/b.md")).toBeVisible();
    expect(screen.getByRole("button", { name: "Try again" })).toBeVisible();
  });

  it("calls the retry endpoint when retrying a failed Vault", async () => {
    let calledRetry = false;
    mockDetail(
      baseVault(
        {
          type: "managed_git",
          repository_url: "https://example.test/notes.git",
          branch: "main",
          mode: "two_way",
          poll_interval_secs: 3600,
        },
        {
          git: "unavailable",
          git_error: {
            code: "managed_git_remote_unreachable",
            message: "unreachable",
            retryable: true,
          },
        },
      ),
      {
        extraRoutes: {
          [`/api/v1/vaults/${VAULT_ID}/retry POST`]: () => {
            calledRetry = true;
            return json({ vault_id: VAULT_ID, schedule: "queued" });
          },
        },
      },
    );
    render(
      <VaultSettingsDetail
        vaultId={VAULT_ID}
        serverIdentity={SERVER_IDENTITY}
        onDisconnect={() => {}}
      />,
    );
    await screen.findByRole("heading", { name: "Field notes" });
    fireEvent.click(screen.getByRole("button", { name: "Try again" }));
    await vi.waitFor(() => expect(calledRetry).toBe(true));
  });

  it("shows no console for a Vault whose behaviour has no remote", async () => {
    mockDetail(baseVault({ type: "local", path: "/notes" }));
    render(
      <VaultSettingsDetail
        vaultId={VAULT_ID}
        serverIdentity={SERVER_IDENTITY}
        onDisconnect={() => {}}
      />,
    );
    await screen.findByRole("heading", { name: "Field notes" });
    expect(screen.queryByText("Healthy")).not.toBeInTheDocument();
  });
});

describe("VaultSettingsDetail — following the collection (#198)", () => {
  it("adopts another writer's change to the same Vault rather than describing a stale one", async () => {
    let git = "ready";
    mockRoutes({
      "/api/v1/vaults": () =>
        json({
          registry_revision: 3,
          collection_revision: 3,
          vaults: [
            baseVault(
              {
                type: "managed_git",
                repository_url: "https://example.test/notes.git",
                branch: "main",
                mode: "two_way",
                poll_interval_secs: 3600,
              },
              {
                git,
                ...(git === "unavailable"
                  ? {
                      git_error: {
                        code: "managed_git_auth",
                        message: "The remote refused the stored sign-in.",
                        retryable: true,
                      },
                    }
                  : {}),
              },
            ),
          ],
          demo_mode: false,
        }),
      "/api/v1/vaults/all/stats": () =>
        json({ data: [{ vault_id: VAULT_ID, note_count: 12 }] }),
      [`/api/v1/vaults/${VAULT_ID}/recent?limit=1`]: () =>
        json({ data: [{ mtime_ns: 0 }] }),
    });

    render(
      <VaultSettingsDetail
        vaultId={VAULT_ID}
        serverIdentity={SERVER_IDENTITY}
        onDisconnect={() => {}}
      />,
    );
    await screen.findByRole("button", { name: "Sync now" });

    git = "unavailable";
    for (const source of window.__hatchdoorEventSources) {
      source.emit(
        "vault-collection-revision",
        JSON.stringify({ collection_revision: 9 }),
      );
    }

    expect(
      await screen.findByRole("button", { name: "Try again" }),
    ).toBeVisible();
    expect(
      screen.queryByRole("button", { name: "Sync now" }),
    ).not.toBeInTheDocument();
  });
});

describe("VaultSettingsIndex — the management entry's recovery state", () => {
  it("shows a red line and a single button for a paused Vault flagged for recovery, and clears it on success", async () => {
    markRecoveryPending(VAULT_ID);
    mockRoutes({
      "/api/v1/vaults": () =>
        json({
          registry_revision: 3,
          collection_revision: 3,
          vaults: [
            baseVault({ type: "local", path: "/notes" }, { enabled: false }),
          ],
          demo_mode: false,
        }),
      "/api/v1/vaults/all/stats": () => json({ data: [] }),
      [`/api/v1/vaults/${VAULT_ID}/enable POST`]: () =>
        json({
          vault: baseVault(
            { type: "local", path: "/notes" },
            { enabled: true },
          ),
          registry_revision: 4,
          collection_revision: 4,
        }),
    });

    render(
      <VaultSettingsIndex selectedVaultId={null} onSelectVault={() => {}} />,
    );

    await screen.findByText("needs attention");
    const recover = screen.getByRole("button", { name: "Try again" });
    fireEvent.click(recover);

    await vi.waitFor(() =>
      expect(screen.queryByText("needs attention")).not.toBeInTheDocument(),
    );
    expect(isRecoveryPending(VAULT_ID)).toBe(false);
  });
});

describe("VaultSettingsIndex — an unreadable registry (#150)", () => {
  it("replaces the whole Vaults group with the documented error block and omits Add a Vault", async () => {
    mockRoutes({
      "/api/v1/vaults": () =>
        json({
          collection_revision: 0,
          vaults: [],
          recovery: {
            code: "vault_registry_recovery_required",
            kind: "corrupt",
            message: "the registry file is not valid JSON",
          },
          demo_mode: false,
        }),
    });

    render(
      <VaultSettingsIndex selectedVaultId={null} onSelectVault={() => {}} />,
    );

    expect(await screen.findByText("Vault Registry Unavailable")).toBeVisible();
    expect(
      screen.getByText(
        "the registry file is not valid JSON Nothing was changed, and your Markdown is untouched.",
      ),
    ).toBeVisible();
    expect(
      screen.queryByRole("button", { name: "Add a Vault" }),
    ).not.toBeInTheDocument();
  });

  it("Try again re-fetches discovery and recovers once the registry reads fine", async () => {
    let broken = true;
    mockRoutes({
      "/api/v1/vaults": () =>
        broken
          ? json({
              collection_revision: 0,
              vaults: [],
              recovery: {
                code: "vault_registry_recovery_required",
                kind: "corrupt",
                message: "the registry file is not valid JSON",
              },
              demo_mode: false,
            })
          : json({
              registry_revision: 0,
              collection_revision: 0,
              vaults: [],
              demo_mode: false,
            }),
      "/api/v1/vaults/all/stats": () => json({ data: [] }),
    });

    render(
      <VaultSettingsIndex selectedVaultId={null} onSelectVault={() => {}} />,
    );
    await screen.findByText("Vault Registry Unavailable");

    broken = false;
    fireEvent.click(screen.getByRole("button", { name: "Try again" }));

    await vi.waitFor(() => {
      expect(
        screen.queryByText("Vault Registry Unavailable"),
      ).not.toBeInTheDocument();
    });
    expect(screen.getByRole("button", { name: "Add a Vault" })).toBeVisible();
  });
});

describe("VaultSettingsIndex — the creation flow entry point (#153)", () => {
  it("opens the creation dialog from the settings-index Add a Vault button", async () => {
    mockRoutes({
      "/api/v1/vaults": () =>
        json({
          registry_revision: 0,
          collection_revision: 0,
          vaults: [],
          demo_mode: false,
        }),
      "/api/v1/vaults/all/stats": () => json({ data: [] }),
    });

    render(
      <VaultSettingsIndex selectedVaultId={null} onSelectVault={() => {}} />,
    );

    fireEvent.click(await screen.findByRole("button", { name: "Add a Vault" }));

    expect(
      await screen.findByRole("dialog", { name: "Add a Vault" }),
    ).toBeVisible();
  });

  it("renders no Add a Vault affordance in demo mode", async () => {
    mockRoutes({
      "/api/v1/vaults": () =>
        json({
          registry_revision: 0,
          collection_revision: 0,
          vaults: [],
          demo_mode: true,
        }),
      "/api/v1/vaults/all/stats": () => json({ data: [] }),
    });

    render(
      <VaultSettingsIndex selectedVaultId={null} onSelectVault={() => {}} />,
    );

    await vi.waitFor(() =>
      expect(
        screen.queryByRole("button", { name: "Add a Vault" }),
      ).not.toBeInTheDocument(),
    );
  });

  it("does not leave an already-open dialog usable once discovery reports demo mode", async () => {
    let resolveDiscovery!: (value: Response) => void;
    mockedApiFetch.mockImplementation(async (input) => {
      const url = String(input);
      if (url === "/api/v1/vaults") {
        return new Promise<Response>((resolve) => {
          resolveDiscovery = resolve;
        });
      }
      if (url === "/api/v1/vaults/all/stats") return json({ data: [] });
      throw new Error(`Unexpected API request: ${url}`);
    });

    render(
      <VaultSettingsIndex selectedVaultId={null} onSelectVault={() => {}} />,
    );

    // `demoMode` starts `false`, so the button renders before discovery
    // resolves — exactly the window a demo visitor could click through.
    fireEvent.click(screen.getByRole("button", { name: "Add a Vault" }));
    expect(screen.getByRole("dialog", { name: "Add a Vault" })).toBeVisible();

    resolveDiscovery(
      json({
        registry_revision: 0,
        collection_revision: 0,
        vaults: [],
        demo_mode: true,
      }),
    );

    await vi.waitFor(() =>
      expect(
        screen.queryByRole("dialog", { name: "Add a Vault" }),
      ).not.toBeInTheDocument(),
    );
  });
});
