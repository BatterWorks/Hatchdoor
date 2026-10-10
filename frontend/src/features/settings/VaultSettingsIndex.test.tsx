import {
  cleanup,
  fireEvent,
  render,
  screen,
  within,
} from "@testing-library/react";
import type { ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { apiFetch } from "../../api/api";
import { CONTEXTUAL_HELP, type ContextualHelp, helpLinkName } from "../help";
import { HelpContext } from "../help/useHelp";
import { VaultSettingsDetail, VaultSettingsIndex } from "./VaultSettingsIndex";
import {
  behaviorOptions,
  buildSourceForBehavior,
  clampPollMinutes,
  describeGitFailure,
  describeRecoveryFailure,
  isRecoveryPending,
  markRecoveryPending,
  recoveryBranchName,
  recoveryBranchUrl,
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
  // Mirrors `collection_capabilities` in `src/vault_runtime.rs`: both flags
  // come from the Vault's Git mode, not from its current status.
  const mode = (source as { mode?: string } | undefined)?.mode;
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
      commit: mode === "local_history" || mode === "two_way",
      sync: mode === "pull_only" || mode === "two_way",
      publish_recovery: false,
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

    const rejected = describeGitFailure({
      code: "managed_git_push_rejected",
      message:
        "managed checkout push was rejected by the remote: pre-receive hook declined",
      retryable: false,
    });
    expect(rejected.label).toBe("push refused");
    expect(rejected.tier).toBe("error");
    expect(rejected.sentence).toContain("refused Hatchdoor's push");
    expect(rejected.files).toBeUndefined();

    const unfinished = describeGitFailure({
      code: "managed_git_operation_in_progress",
      message: "unfinished merge",
      retryable: false,
      detail: { kind: "affected_paths", paths: ["notes/Home.md"], total: 1 },
    });
    expect(unfinished.label).toBe("unfinished merge");
    expect(unfinished.tier).toBe("error");
    expect(unfinished.sentence).toContain("part-way through a merge");
    expect(unfinished.files).toEqual(["notes/Home.md"]);

    const unknown = describeGitFailure({
      code: "managed_git_not_remote",
      message: "not a remote vault",
      retryable: false,
    });
    expect(unknown.sentence).toContain("not a remote vault");
  });

  it("names the recovery branch from the last publish, else from the configured branch", () => {
    const source = {
      type: "managed_git",
      repository_url: "https://example.test/notes.git",
      branch: "main",
      mode: "two_way",
      poll_interval_secs: 3600,
    };
    expect(recoveryBranchName(baseVault(source) as never)).toBe(
      `hatchdoor-recovery/main/${VAULT_ID}`,
    );
    expect(
      recoveryBranchName(
        baseVault(
          { ...source, branch: undefined },
          {
            recovery_branch: { branch: `hatchdoor-recovery/trunk/${VAULT_ID}` },
          },
        ) as never,
      ),
    ).toBe(`hatchdoor-recovery/trunk/${VAULT_ID}`);
    expect(
      recoveryBranchName(baseVault({ ...source, branch: undefined }) as never),
    ).toBeNull();
  });

  it("links a recovery branch only on an HTTPS remote without credentials", () => {
    const branch = `hatchdoor-recovery/main/${VAULT_ID}`;
    const source = (repository_url: string) => ({
      type: "managed_git" as const,
      repository_url,
      mode: "two_way" as const,
      poll_interval_secs: 3600,
    });
    expect(
      recoveryBranchUrl(source("https://github.com/owner/notes.git"), branch),
    ).toBe(`https://github.com/owner/notes/tree/${branch}`);
    expect(
      recoveryBranchUrl(
        source("https://git.example.test/owner/notes/"),
        branch,
      ),
    ).toBe(`https://git.example.test/owner/notes/tree/${branch}`);
    expect(
      recoveryBranchUrl(source("http://git.example.test/notes.git"), branch),
    ).toBeNull();
    expect(
      recoveryBranchUrl(source("https://me:secret@example.test/n.git"), branch),
    ).toBeNull();
    expect(recoveryBranchUrl({ type: "local", path: "/n" }, branch)).toBeNull();
  });

  it("says why a publish published nothing", () => {
    expect(
      describeRecoveryFailure({
        code: "managed_git_recovery_diverged",
        message: "diverged",
        retryable: false,
      }),
    ).toMatch(/Someone added commits to this branch/);
    expect(
      describeRecoveryFailure({
        code: "managed_git_recovery_push_rejected",
        message: "the remote refused the recovery branch: protected",
        retryable: false,
      }),
    ).toMatch(/refused the branch.*protected/);
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

    // The failure notice and the red-line recovery state are both alerts.
    expect(await screen.findAllByRole("alert")).toHaveLength(2);
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

  it("offers Commit now, and no talk of a remote, for a Local history Vault", async () => {
    // The console used to render for any Git-backed Vault as though every one
    // of them synced, so a Vault with no remote was told its "Git sync is
    // healthy" and offered a Sync now the backend could only refuse (#267).
    mockDetail(
      baseVault(
        {
          type: "existing_git",
          repository_path: "/vaults/field-notes",
          mode: "local_history",
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
    expect(screen.getByRole("button", { name: "Commit now" })).toBeVisible();
    expect(
      screen.queryByRole("button", { name: "Sync now" }),
    ).not.toBeInTheDocument();
    expect(
      screen.getByText("This Vault's Git history is up to date."),
    ).toBeVisible();
    expect(
      screen.queryByText("This Vault's Git sync is healthy."),
    ).not.toBeInTheDocument();
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

  const conflictedSource = {
    type: "managed_git",
    repository_url: "https://example.test/owner/notes.git",
    branch: "main",
    mode: "two_way",
    poll_interval_secs: 3600,
  };
  const conflictError = {
    code: "managed_git_conflict",
    message: "conflict",
    retryable: false,
    detail: { kind: "affected_paths", paths: ["notes/a.md"], total: 1 },
  };
  function conflictedVault(overrides: Record<string, unknown> = {}) {
    const vault = baseVault(conflictedSource, {
      git: "unavailable",
      git_error: conflictError,
      ...overrides,
    });
    vault.capabilities = { ...vault.capabilities, publish_recovery: true };
    return vault;
  }

  it("offers to publish a conflicted Vault's side to its recovery branch (ADR-30)", async () => {
    let published = false;
    mockDetail(conflictedVault(), {
      extraRoutes: {
        [`/api/v1/vaults/${VAULT_ID}/recovery-branch POST`]: () => {
          published = true;
          return json({ vault_id: VAULT_ID, schedule: "queued" });
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
    expect(
      screen.getByText(`hatchdoor-recovery/main/${VAULT_ID}`),
    ).toBeVisible();
    expect(screen.getByRole("button", { name: "Copy" })).toBeVisible();
    expect(
      screen.queryByRole("link", { name: "Open on the Git host" }),
    ).not.toBeInTheDocument();
    fireEvent.click(
      screen.getByRole("button", { name: "Publish my side to a branch" }),
    );
    await vi.waitFor(() => expect(published).toBe(true));
    expect(
      await screen.findByText(/Publishing this Vault's side/),
    ).toBeVisible();
  });

  it("shows the published commit and a link once the branch is on the remote", async () => {
    mockDetail(
      conflictedVault({
        recovery_branch: {
          branch: `hatchdoor-recovery/main/${VAULT_ID}`,
          published_commit: "0123456789abcdef0123456789abcdef01234567",
          conflicting_commit: "fedcba9876543210fedcba9876543210fedcba98",
          published_at: new Date().toISOString(),
        },
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
    expect(screen.getByText(/Published 0123456 just now/)).toBeVisible();
    expect(
      screen.getByRole("link", { name: "Open on the Git host" }),
    ).toHaveAttribute(
      "href",
      `https://example.test/owner/notes/tree/hatchdoor-recovery/main/${VAULT_ID}`,
    );
    expect(screen.getByRole("button", { name: "Publish again" })).toBeVisible();
  });

  it("says why a publish was refused", async () => {
    mockDetail(
      conflictedVault({
        recovery_branch: {
          branch: `hatchdoor-recovery/main/${VAULT_ID}`,
          error: {
            code: "managed_git_recovery_diverged",
            message: "diverged",
            retryable: false,
          },
        },
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
    expect(
      screen.getByText(/Someone added commits to this branch/),
    ).toBeVisible();
  });

  it("offers no recovery branch for a failure that is not a conflict", async () => {
    mockDetail(
      baseVault(conflictedSource, {
        git: "unavailable",
        git_error: {
          code: "managed_git_remote_unreachable",
          message: "unreachable",
          retryable: true,
        },
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
    expect(
      screen.queryByRole("button", { name: /Publish/ }),
    ).not.toBeInTheDocument();
  });

  it("lists the conflicted files of an unfinished merge the checkout was left in", async () => {
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
            code: "managed_git_operation_in_progress",
            message:
              "managed checkout has an unfinished merge with conflicts in: notes/Home.md",
            retryable: false,
            detail: {
              kind: "affected_paths",
              paths: ["notes/Home.md"],
              total: 1,
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
    expect(screen.getByText(/part-way through a merge/i)).toBeVisible();
    expect(screen.getByText("notes/Home.md")).toBeVisible();
    expect(
      screen.queryByText(/Something unexpected stopped/i),
    ).not.toBeInTheDocument();
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

  it("still adopts a change that arrived while its own round trip was in flight", async () => {
    markRecoveryPending(VAULT_ID);
    let name = "Field notes";
    let resolveEnable: ((value: Response) => void) | null = null;
    mockRoutes({
      "/api/v1/vaults": () =>
        json({
          registry_revision: 3,
          collection_revision: 3,
          vaults: [
            baseVault(
              { type: "local", path: "/notes" },
              { enabled: false, name },
            ),
          ],
          demo_mode: false,
        }),
      "/api/v1/vaults/all/stats": () =>
        json({ data: [{ vault_id: VAULT_ID, note_count: 12 }] }),
      [`/api/v1/vaults/${VAULT_ID}/recent?limit=1`]: () =>
        json({ data: [{ mtime_ns: 0 }] }),
      [`/api/v1/vaults/${VAULT_ID}/enable POST`]: () =>
        new Promise<Response>((resolve) => {
          resolveEnable = resolve;
        }),
    });

    render(
      <VaultSettingsDetail
        vaultId={VAULT_ID}
        serverIdentity={SERVER_IDENTITY}
        onDisconnect={() => {}}
      />,
    );
    await screen.findByRole("heading", { name: "Field notes" });

    // The recovery round trip sets `busy`, and the page deliberately shows its
    // own in-flight state while it runs.
    fireEvent.click(
      screen.getByRole("button", { name: "Try to bring this Vault back" }),
    );
    await vi.waitFor(() => expect(resolveEnable).not.toBeNull());

    // Someone else renames the Vault mid-round-trip. The record must not be
    // silently consumed: the collection keeps a Vault's identity across a
    // refresh that finds nothing new, so a record dropped here would never be
    // offered again and this page would describe a Vault the index disagrees
    // with for the rest of the visit.
    name = "Renamed elsewhere";
    for (const source of window.__hatchdoorEventSources) {
      source.emit(
        "vault-collection-revision",
        JSON.stringify({ collection_revision: 9 }),
      );
    }
    await vi.waitFor(() =>
      expect(
        screen.getByRole("heading", { name: "Field notes" }),
      ).toBeVisible(),
    );

    // The round trip answers without a record of its own, so nothing competes
    // with the collection for the last word.
    resolveEnable!(json({ registry_revision: 4, collection_revision: 4 }));

    expect(
      await screen.findByRole("heading", { name: "Renamed elsewhere" }),
    ).toBeVisible();
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

  // The index is `position: sticky`, so a dialog left inside it stacks only
  // within the sidebar and the page's main column paints over it (#448).
  it("mounts the creation dialog outside the sticky settings index", async () => {
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

    const { container } = render(
      <aside className="settings-index">
        <VaultSettingsIndex selectedVaultId={null} onSelectVault={() => {}} />
      </aside>,
    );

    fireEvent.click(await screen.findByRole("button", { name: "Add a Vault" }));

    const dialog = await screen.findByRole("dialog", { name: "Add a Vault" });
    expect(container.querySelector(".settings-index")).not.toContainElement(
      dialog,
    );
    expect(dialog.closest(".settings-modal-back")?.parentElement).toBe(
      document.body,
    );
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

describe("VaultSettingsDetail — the note count in the blurb (#333)", () => {
  function mockStats(stats: () => Response) {
    // The stats route is listed first: the discovery pattern is a prefix of it.
    mockRoutes({
      "/api/v1/vaults/all/stats": stats,
      "/api/v1/vaults": () =>
        json({
          registry_revision: 3,
          collection_revision: 3,
          vaults: [baseVault({ type: "local", path: "/notes" })],
          demo_mode: false,
        }),
      [`/api/v1/vaults/${VAULT_ID}/recent?limit=1`]: () =>
        json({ data: [{ mtime_ns: 0 }] }),
    });
  }

  function renderDetail() {
    render(
      <VaultSettingsDetail
        vaultId={VAULT_ID}
        serverIdentity={SERVER_IDENTITY}
        onDisconnect={() => {}}
      />,
    );
  }

  it("shows the count the stats read supplied", async () => {
    mockStats(() => json({ data: [{ vault_id: VAULT_ID, note_count: 12 }] }));
    renderDetail();
    await screen.findByRole("heading", { name: "Field notes" });
    expect(await screen.findByText(/· 12 notes ·/)).toBeVisible();
  });

  it("marks the count unknown, never 0, when a partial read left this Vault out", async () => {
    mockStats(() =>
      json({
        data: [],
        partial: true,
        participants: [{ vault_id: VAULT_ID, state: "unavailable" }],
      }),
    );
    renderDetail();
    await screen.findByRole("heading", { name: "Field notes" });
    await vi.waitFor(() =>
      expect(mockedApiFetch).toHaveBeenCalledWith("/api/v1/vaults/all/stats"),
    );
    const marker = await screen.findByLabelText("Note count not known");
    expect(marker).toHaveTextContent("–");
    expect(marker.parentElement).toHaveTextContent(/– notes/);
    expect(screen.queryByText(/\b0 notes\b/)).not.toBeInTheDocument();
  });
});

describe("VaultSettingsDetail — a registry that moved elsewhere (#338)", () => {
  const conflict = (expected: unknown, current: number) =>
    new Response(
      JSON.stringify({
        code: "registry_revision_conflict",
        message: `expected registry revision ${String(expected)}, current revision is ${current}`,
        retryable: true,
      }),
      { status: 409, headers: { "content-type": "application/json" } },
    );

  /** A registry whose revision a test can move, as another tab or an MCP
   * agent would, with every guarded route refusing a stale revision the way
   * `src/vault_management.rs` does. */
  function mockMovingRegistry(
    onPatch?: (body: Record<string, unknown>) => void,
  ) {
    const registry = { revision: 3 };
    const vault = baseVault({ type: "local", path: "/notes" });
    const expectedFrom = (init: RequestInit | undefined, url: string) =>
      init?.body
        ? (JSON.parse(String(init.body)) as Record<string, unknown>)
            .expected_registry_revision
        : Number(
            new URL(url, "http://x").searchParams.get(
              "expected_registry_revision",
            ),
          );
    const calls: string[] = [];
    mockedApiFetch.mockImplementation(async (input, init) => {
      const url = String(input);
      const method = init?.method ?? "GET";
      calls.push(`${method} ${url}`);
      if (url === "/api/v1/vaults/all/stats") return json({ data: [] });
      if (url.startsWith(`/api/v1/vaults/${VAULT_ID}/recent`))
        return json({ data: [] });
      if (url === "/api/v1/vaults" && method === "GET")
        return json({
          registry_revision: registry.revision,
          collection_revision: registry.revision,
          vaults: [vault],
          demo_mode: false,
        });
      if (url.startsWith(`/api/v1/vaults/${VAULT_ID}`) && method !== "GET") {
        const expected = expectedFrom(init, url);
        if (expected !== registry.revision)
          return conflict(expected, registry.revision);
        if (method === "PATCH")
          onPatch?.(JSON.parse(String(init?.body)) as Record<string, unknown>);
        registry.revision += 1;
        return json({
          vault: {
            ...vault,
            enabled: !url.includes("/disable"),
          },
          registry_revision: registry.revision,
          collection_revision: registry.revision,
        });
      }
      throw new Error(`Unexpected API request: ${method} ${url}`);
    });
    return { registry, calls };
  }

  function renderDetail() {
    render(
      <VaultSettingsDetail
        vaultId={VAULT_ID}
        serverIdentity={SERVER_IDENTITY}
        onDisconnect={() => {}}
      />,
    );
  }

  it("pauses against the revision current at the click, not the one the page opened with", async () => {
    const { registry, calls } = mockMovingRegistry();
    renderDetail();
    await screen.findByRole("heading", { name: "Field notes" });

    registry.revision = 7; // another tab edited a different Vault
    fireEvent.click(screen.getByRole("button", { name: "Pause Vault" }));

    await screen.findByText("Saved.");
    expect(calls).toContain(
      `POST /api/v1/vaults/${VAULT_ID}/disable?expected_registry_revision=7`,
    );
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("says a Save conflict in words, as an alert, and lets the next Save through", async () => {
    let patched: Record<string, unknown> | undefined;
    const { registry } = mockMovingRegistry((body) => (patched = body));
    renderDetail();
    await screen.findByRole("heading", { name: "Field notes" });

    registry.revision = 7;
    fireEvent.change(screen.getByLabelText("Vault name"), {
      target: { value: "Renamed" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save Vault" }));

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent(/changed elsewhere/);
    expect(alert).not.toHaveTextContent(/registry revision/);
    expect(patched).toBeUndefined();
    // The typed edit survives the refusal.
    expect(screen.getByLabelText("Vault name")).toHaveValue("Renamed");

    fireEvent.click(screen.getByRole("button", { name: "Save Vault" }));
    await screen.findByText("Saved.");
    expect(patched?.expected_registry_revision).toBe(7);
    expect(patched?.name).toBe("Renamed");
  });
});

describe("VaultSettingsDetail — a Git behaviour switch clears what leaves the screen (#338)", () => {
  const twoWay = {
    type: "existing_git",
    repository_path: "/notes",
    repository_url: "https://example.test/notes.git",
    branch: "main",
    mode: "two_way",
    poll_interval_secs: 3600,
  };

  it("never sends a token typed for a remote behaviour once the choice has no remote", async () => {
    let patched: Record<string, unknown> | undefined;
    mockDetail(baseVault(twoWay, { credential_configured: true }), {
      onPatch: (body) => (patched = body),
    });
    render(
      <VaultSettingsDetail
        vaultId={VAULT_ID}
        serverIdentity={SERVER_IDENTITY}
        onDisconnect={() => {}}
      />,
    );

    await screen.findByRole("heading", { name: "Field notes" });
    fireEvent.change(screen.getByLabelText("Repository access token"), {
      target: { value: "s3cr3t" },
    });
    fireEvent.change(screen.getByLabelText("Sync schedule in minutes"), {
      target: { value: "5" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Local history" }));
    expect(
      screen.queryByLabelText("Repository access token"),
    ).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Save Vault" }));

    await screen.findByText("Saved.");
    expect(patched?.https_credentials).toEqual({ action: "remove" });
    expect(JSON.stringify(patched)).not.toContain("s3cr3t");
    expect(
      (patched?.source as { poll_interval_secs?: number }).poll_interval_secs,
    ).toBe(3600);
  });

  it("brings the saved sign-in back, with an empty token field, when the choice returns to a remote", async () => {
    mockDetail(baseVault(twoWay, { credential_configured: true }));
    render(
      <VaultSettingsDetail
        vaultId={VAULT_ID}
        serverIdentity={SERVER_IDENTITY}
        onDisconnect={() => {}}
      />,
    );

    await screen.findByRole("heading", { name: "Field notes" });
    fireEvent.change(screen.getByLabelText("Repository access token"), {
      target: { value: "s3cr3t" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Local history" }));
    fireEvent.click(screen.getByRole("button", { name: "Two-way" }));

    expect(screen.getByLabelText("Repository access token")).toHaveValue("");
    expect(
      screen.getByRole("button", { name: "Access token" }),
    ).toHaveAttribute("aria-pressed", "true");
  });
});

describe("VaultSettingsDetail — the last change in the blurb (#338)", () => {
  it("never prints the previous Vault's date under a Vault whose read was refused", async () => {
    const OTHER_ID = "00000000-0000-4000-8000-000000000002";
    const first = baseVault({ type: "local", path: "/notes" });
    const second = {
      ...baseVault({ type: "local", path: "/other" }),
      vault_id: OTHER_ID,
      name: "Other notes",
      search: "indexing",
    };
    mockedApiFetch.mockImplementation(async (input) => {
      const url = String(input);
      if (url === "/api/v1/vaults/all/stats") return json({ data: [] });
      if (url === `/api/v1/vaults/${VAULT_ID}/recent?limit=1`)
        return json({ data: [{ mtime_ns: Date.UTC(2020, 0, 15) * 1e6 }] });
      if (url === `/api/v1/vaults/${OTHER_ID}/recent?limit=1`)
        return new Response("{}", { status: 503 });
      if (url === "/api/v1/vaults")
        return json({
          registry_revision: 3,
          collection_revision: 3,
          vaults: [first, second],
          demo_mode: false,
        });
      throw new Error(`Unexpected API request: ${url}`);
    });

    const { rerender } = render(
      <VaultSettingsDetail
        vaultId={VAULT_ID}
        serverIdentity={SERVER_IDENTITY}
        onDisconnect={() => {}}
      />,
    );
    await screen.findByText(/changed \d/);

    rerender(
      <VaultSettingsDetail
        vaultId={OTHER_ID}
        serverIdentity={SERVER_IDENTITY}
        onDisconnect={() => {}}
      />,
    );
    await screen.findByText(/last change unavailable/);
    expect(screen.queryByText(/changed \d/)).not.toBeInTheDocument();
  });
});

describe("Settings modals (#338)", () => {
  it("focuses the confirmation, keeps Tab inside it, and closes it on Escape", async () => {
    mockDetail(baseVault({ type: "local", path: "/notes" }));
    render(
      <VaultSettingsDetail
        vaultId={VAULT_ID}
        serverIdentity={SERVER_IDENTITY}
        onDisconnect={() => {}}
      />,
    );

    await screen.findByRole("heading", { name: "Field notes" });
    fireEvent.click(screen.getByRole("button", { name: "Local history" }));
    const save = screen.getByRole("button", { name: "Save Vault" });
    save.focus();
    fireEvent.click(save);

    const dialog = await screen.findByRole("dialog", {
      name: "Before this is saved",
    });
    const cancel = within(dialog).getByRole("button", { name: "Cancel" });
    const goAhead = within(dialog).getByRole("button", { name: "Go ahead" });
    expect(cancel).toHaveFocus();

    goAhead.focus();
    fireEvent.keyDown(goAhead, { key: "Tab" });
    expect(cancel).toHaveFocus();
    fireEvent.keyDown(cancel, { key: "Tab", shiftKey: true });
    expect(goAhead).toHaveFocus();

    fireEvent.keyDown(goAhead, { key: "Escape" });
    expect(
      screen.queryByRole("dialog", { name: "Before this is saved" }),
    ).not.toBeInTheDocument();
    expect(save).toHaveFocus();
  });
});

describe("How does this work? links (#423)", () => {
  const openHelp = vi.fn();

  function withHelp(node: ReactNode) {
    return (
      <HelpContext.Provider
        value={{ openHelp, closeHelp: () => {}, isOpen: false }}
      >
        {node}
      </HelpContext.Provider>
    );
  }

  async function renderDetail(vault: unknown) {
    mockDetail(vault);
    const view = render(
      withHelp(
        <VaultSettingsDetail
          vaultId={VAULT_ID}
          serverIdentity={SERVER_IDENTITY}
          onDisconnect={() => {}}
        />,
      ),
    );
    await screen.findByRole("heading", { name: "Field notes" });
    return view;
  }

  function clickLinkIn(container: Element | null, to: ContextualHelp) {
    expect(container).not.toBeNull();
    fireEvent.click(
      within(container as HTMLElement).getByRole("button", {
        name: helpLinkName(to),
      }),
    );
  }

  function expectOpened(target: { page: string; heading?: string }) {
    expect(openHelp).toHaveBeenLastCalledWith(
      target.page,
      "heading" in target ? target.heading : undefined,
    );
  }

  const local = { type: "local", path: "/notes" };
  const twoWay = {
    type: "managed_git",
    repository_url: "https://example.test/notes.git",
    branch: "main",
    vault_subdirectory: null,
    mode: "two_way",
    poll_interval_secs: 3600,
  };

  it("links a healthy Vault's page to managing Vaults", async () => {
    const { container } = await renderDetail(baseVault(local));
    clickLinkIn(
      container.querySelector(".settings-vault-condition"),
      CONTEXTUAL_HELP.vaultSettings,
    );
    expectOpened(CONTEXTUAL_HELP.vaultSettings);
  });

  it("links a paused Vault to pausing and resuming", async () => {
    const { container } = await renderDetail(
      baseVault(local, { enabled: false, activation: "disabled" }),
    );
    clickLinkIn(
      container.querySelector(".settings-vault-condition"),
      CONTEXTUAL_HELP.vaultPaused,
    );
    expectOpened(CONTEXTUAL_HELP.vaultPaused);
  });

  it("links a missing folder to the permissions section", async () => {
    const { container } = await renderDetail(
      baseVault(local, {
        activation: "unavailable",
        local_content: "unavailable",
        search: "unavailable",
        activation_error: {
          code: "vault_path_unavailable",
          message: "No such file or directory (os error 2)",
          retryable: true,
        },
      }),
    );
    clickLinkIn(
      container.querySelector(".settings-vault-condition"),
      CONTEXTUAL_HELP.vaultFolder,
    );
    expectOpened(CONTEXTUAL_HELP.vaultFolder);
  });

  it("links any other unavailable or stale Vault to the bad-state section", async () => {
    const { container } = await renderDetail(
      baseVault(local, {
        search: "stale",
        search_error: {
          code: "vault_index_failed",
          message: "Indexing could not be completed.",
          retryable: true,
        },
      }),
    );
    clickLinkIn(
      container.querySelector(".settings-vault-condition"),
      CONTEXTUAL_HELP.vaultUnavailable,
    );
    expectOpened(CONTEXTUAL_HELP.vaultUnavailable);
  });

  it("links a healthy Git console to the Git-backed Vault guide", async () => {
    const { container } = await renderDetail(
      baseVault(twoWay, { git: "ready" }),
    );
    clickLinkIn(
      container.querySelector(".settings-git-console"),
      CONTEXTUAL_HELP.gitSetup,
    );
    expectOpened(CONTEXTUAL_HELP.gitSetup);
  });

  it("links a failing sync to the Git section, and only from the console", async () => {
    const { container } = await renderDetail(
      baseVault(twoWay, {
        git: "unavailable",
        git_error: {
          code: "managed_git_remote_unreachable",
          message: "could not reach the remote",
          retryable: true,
        },
      }),
    );
    clickLinkIn(
      container.querySelector(".settings-git-console"),
      CONTEXTUAL_HELP.gitFailing,
    );
    expectOpened(CONTEXTUAL_HELP.gitFailing);
    expect(
      within(
        container.querySelector(".settings-vault-condition") as HTMLElement,
      ).queryByRole("button", { name: /^How does this work\?/ }),
    ).not.toBeInTheDocument();
  });

  it("links a sync conflict to resolving it", async () => {
    const { container } = await renderDetail(
      baseVault(twoWay, {
        git: "unavailable",
        git_error: {
          code: "managed_git_conflict",
          message: "notes changed on both sides",
          retryable: false,
        },
      }),
    );
    clickLinkIn(
      container.querySelector(".settings-git-console"),
      CONTEXTUAL_HELP.gitConflict,
    );
    expectOpened(CONTEXTUAL_HELP.gitConflict);
  });

  it("links the registry recovery block in the Vault index", async () => {
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
      withHelp(
        <VaultSettingsIndex selectedVaultId={null} onSelectVault={() => {}} />,
      ),
    );
    await screen.findByText("Vault Registry Unavailable");
    clickLinkIn(document.body, CONTEXTUAL_HELP.registryRecovery);
    expectOpened(CONTEXTUAL_HELP.registryRecovery);
  });
});
