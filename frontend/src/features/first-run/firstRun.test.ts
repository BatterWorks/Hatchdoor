import {
  afterEach,
  beforeEach,
  describe,
  expect,
  it,
  vi,
  type Mock,
} from "vitest";

import {
  clientConfigs,
  connectAgent,
  DISMISSED_KEY,
  dismissFirstRun,
  fetchAgentSetup,
  fetchFreshInstall,
  mcpAddress,
  recordSearchResults,
  reopenFirstRun,
  resetFirstRunForTests,
  SEARCH_KEY,
  setUsageReport,
  shouldShowFirstRun,
  TOKEN_PLACEHOLDER,
  type AgentSetup,
} from "./firstRun";

type Call = { url: string; method: string; body: unknown };

/** Answers each request by `METHOD url`, recording what was sent. */
function serve(routes: Record<string, { status?: number; body: unknown }>): {
  fetchMock: Mock;
  calls: Call[];
} {
  const calls: Call[] = [];
  const fetchMock = vi
    .spyOn(globalThis, "fetch")
    .mockImplementation(async (input, init) => {
      const url = String(input);
      const method = init?.method ?? "GET";
      calls.push({
        url,
        method,
        body: init?.body ? JSON.parse(String(init.body)) : undefined,
      });
      const route = routes[`${method} ${url}`];
      if (!route) return new Response("{}", { status: 404 });
      return new Response(JSON.stringify(route.body), {
        status: route.status ?? 200,
      });
    }) as unknown as Mock;
  return { fetchMock, calls };
}

function setting(
  key: string,
  value: string | null,
  extra: Partial<{ locked: string | null; configured: boolean }> = {},
) {
  return { key, value, locked: null, ...extra };
}

function settingsBody(
  overrides: Record<string, ReturnType<typeof setting>> = {},
  lastAgent: unknown = null,
) {
  const base = {
    HATCHDOOR_MCP_ENABLED: setting("HATCHDOOR_MCP_ENABLED", "false"),
    HATCHDOOR_MCP_WRITE_ENABLED: setting(
      "HATCHDOOR_MCP_WRITE_ENABLED",
      "false",
    ),
    HATCHDOOR_MCP_BEARER_TOKEN: setting("HATCHDOOR_MCP_BEARER_TOKEN", null, {
      configured: false,
    }),
    HATCHDOOR_PUBLIC_URL: setting("HATCHDOOR_PUBLIC_URL", ""),
    HATCHDOOR_UPDATE_CHECK_ENABLED: setting(
      "HATCHDOOR_UPDATE_CHECK_ENABLED",
      "false",
    ),
    ...overrides,
  };
  return { settings: Object.values(base), last_agent: lastAgent };
}

const OFF: AgentSetup = {
  mcpEnabled: false,
  mcpLocked: false,
  writesEnabled: false,
  writesLocked: false,
  tokenLocked: false,
  publicUrl: "",
  updateCheckEnabled: false,
  updateCheckLocked: false,
  usageReportEnabled: false,
  usageReportLocked: false,
  lastAgent: null,
};

beforeEach(() => {
  window.localStorage.clear();
  resetFirstRunForTests();
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe("shouldShowFirstRun", () => {
  const base = {
    demoMode: false,
    freshInstall: true,
    dismissed: false,
    reopened: false,
  };

  it("shows on a fresh install until dismissed", () => {
    expect(shouldShowFirstRun(base)).toBe(true);
    expect(shouldShowFirstRun({ ...base, dismissed: true })).toBe(false);
  });

  it("never shows on its own to an upgraded install", () => {
    expect(shouldShowFirstRun({ ...base, freshInstall: false })).toBe(false);
  });

  it("shows on any install once reopened from Help", () => {
    expect(
      shouldShowFirstRun({ ...base, freshInstall: false, reopened: true }),
    ).toBe(true);
  });

  it("never shows in demo mode, even reopened", () => {
    expect(shouldShowFirstRun({ ...base, demoMode: true })).toBe(false);
    expect(
      shouldShowFirstRun({ ...base, demoMode: true, reopened: true }),
    ).toBe(false);
  });
});

describe("browser memory", () => {
  it("remembers a dismissal across loads, and forgets it when reopened", () => {
    dismissFirstRun();
    expect(window.localStorage.getItem(DISMISSED_KEY)).toBe("1");
    resetFirstRunForTests();
    reopenFirstRun();
    expect(window.localStorage.getItem(DISMISSED_KEY)).toBeNull();
  });

  it("keeps the first search that found something, and ignores empty ones", () => {
    recordSearchResults("  ", 3);
    recordSearchResults("nothing", 0);
    expect(window.localStorage.getItem(SEARCH_KEY)).toBeNull();
    recordSearchResults("recipes", 12);
    recordSearchResults("later", 4);
    expect(JSON.parse(window.localStorage.getItem(SEARCH_KEY)!)).toEqual({
      query: "recipes",
      count: 12,
    });
  });

  it("does not throw when storage does", () => {
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new DOMException("blocked", "SecurityError");
    });
    expect(() => dismissFirstRun()).not.toThrow();
    expect(() => recordSearchResults("recipes", 1)).not.toThrow();
  });
});

describe("fetchFreshInstall", () => {
  it("reads the server's fresh_install flag", async () => {
    serve({
      "GET /api/v1/whats-new": { body: { fresh_install: true, releases: [] } },
    });
    expect(await fetchFreshInstall()).toBe(true);
  });

  it("treats a failed read as not fresh", async () => {
    serve({ "GET /api/v1/whats-new": { status: 500, body: {} } });
    expect(await fetchFreshInstall()).toBe(false);
  });
});

describe("fetchAgentSetup", () => {
  it("reads MCP, writes, the password, the update check and the last agent", async () => {
    serve({
      "GET /api/settings": {
        body: settingsBody(
          {
            HATCHDOOR_MCP_ENABLED: setting("HATCHDOOR_MCP_ENABLED", "true"),
            HATCHDOOR_MCP_BEARER_TOKEN: setting(
              "HATCHDOOR_MCP_BEARER_TOKEN",
              null,
              {
                configured: true,
                locked: "environment",
              },
            ),
            HATCHDOOR_PUBLIC_URL: setting(
              "HATCHDOOR_PUBLIC_URL",
              "https://notes.example.com",
            ),
          },
          { name: "Claude Code", connected_at: "2026-10-03T09:00:00Z" },
        ),
      },
    });
    const result = await fetchAgentSetup();
    expect(result).toEqual({
      ok: true,
      value: {
        ...OFF,
        mcpEnabled: true,
        tokenLocked: true,
        publicUrl: "https://notes.example.com",
        lastAgent: {
          name: "Claude Code",
          connected_at: "2026-10-03T09:00:00Z",
        },
      },
    });
  });
});

describe("the usage report setting", () => {
  it("is read as on and locked when the configuration file holds it on", async () => {
    serve({
      "GET /api/settings": {
        body: settingsBody({
          HATCHDOOR_USAGE_REPORT_ENABLED: setting(
            "HATCHDOOR_USAGE_REPORT_ENABLED",
            "true",
            { locked: "environment" },
          ),
        }),
      },
    });
    const result = await fetchAgentSetup();
    expect(result).toEqual({
      ok: true,
      value: { ...OFF, usageReportEnabled: true, usageReportLocked: true },
    });
  });

  it("is saved alone, leaving the update check as it was", async () => {
    const { calls } = serve({
      "PATCH /api/settings": {
        body: settingsBody({
          HATCHDOOR_USAGE_REPORT_ENABLED: setting(
            "HATCHDOOR_USAGE_REPORT_ENABLED",
            "true",
          ),
        }),
      },
    });
    const result = await setUsageReport(true);
    expect(result).toEqual({
      ok: true,
      value: { ...OFF, usageReportEnabled: true },
    });
    expect(calls.find((call) => call.method === "PATCH")?.body).toEqual({
      updates: { HATCHDOOR_USAGE_REPORT_ENABLED: "true" },
      confirm: [],
    });
  });
});

describe("connectAgent", () => {
  it("turns MCP on with writes off and a new password, through the Settings requests", async () => {
    const { calls } = serve({
      "POST /api/settings/mcp-token/generate": {
        body: { value: "fresh-token" },
      },
      "PATCH /api/settings": {
        body: settingsBody({
          HATCHDOOR_MCP_ENABLED: setting("HATCHDOOR_MCP_ENABLED", "true"),
          HATCHDOOR_MCP_BEARER_TOKEN: setting(
            "HATCHDOOR_MCP_BEARER_TOKEN",
            null,
            {
              configured: true,
            },
          ),
        }),
      },
    });
    const result = await connectAgent(OFF);
    expect(result.ok && result.value.token).toBe("fresh-token");
    expect(result.ok && result.value.setup.mcpEnabled).toBe(true);
    const patch = calls.find((call) => call.method === "PATCH");
    expect(patch?.body).toEqual({
      updates: {
        HATCHDOOR_MCP_ENABLED: "true",
        HATCHDOOR_MCP_WRITE_ENABLED: "false",
        HATCHDOOR_MCP_BEARER_TOKEN: "fresh-token",
      },
      confirm: [],
    });
  });

  it("never sends writes on, even when they were on", async () => {
    const { calls } = serve({
      "POST /api/settings/mcp-token/generate": { body: { value: "t" } },
      "PATCH /api/settings": { body: settingsBody() },
    });
    await connectAgent({ ...OFF, writesEnabled: true });
    const patch = calls.find((call) => call.method === "PATCH");
    expect(
      (patch?.body as { updates: Record<string, string> }).updates
        .HATCHDOOR_MCP_WRITE_ENABLED,
    ).toBe("false");
  });

  it("leaves settings held by the configuration file alone", async () => {
    const { calls } = serve({
      "PATCH /api/settings": { body: settingsBody() },
    });
    const result = await connectAgent({
      ...OFF,
      tokenLocked: true,
      writesLocked: true,
    });
    expect(result.ok && result.value.token).toBeNull();
    expect(calls.some((call) => call.url.includes("generate"))).toBe(false);
    const patch = calls.find((call) => call.method === "PATCH");
    expect(patch?.body).toEqual({
      updates: { HATCHDOOR_MCP_ENABLED: "true" },
      confirm: [],
    });
  });

  it("refuses when the configuration file holds writes on, rather than connect an agent that can change notes", async () => {
    const { calls } = serve({});
    const result = await connectAgent({
      ...OFF,
      writesLocked: true,
      writesEnabled: true,
    });
    expect(result.ok).toBe(false);
    expect(calls).toHaveLength(0);
  });

  it("refuses when the configuration file keeps MCP off", async () => {
    const { calls } = serve({});
    const result = await connectAgent({ ...OFF, mcpLocked: true });
    expect(result.ok).toBe(false);
    expect(calls).toHaveLength(0);
  });

  it("reports a refused save in the server's words", async () => {
    serve({
      "POST /api/settings/mcp-token/generate": { body: { value: "t" } },
      "PATCH /api/settings": {
        status: 400,
        body: { fields: [{ key: null, message: "Not allowed." }] },
      },
    });
    expect(await connectAgent(OFF)).toEqual({
      ok: false,
      message: "Not allowed.",
    });
  });
});

describe("client configs", () => {
  it("builds the MCP address from the public address, else this page's", () => {
    expect(mcpAddress("", "http://192.168.1.20:42824")).toBe(
      "http://192.168.1.20:42824/mcp",
    );
    expect(mcpAddress("https://notes.example.com/", "http://x")).toBe(
      "https://notes.example.com/mcp",
    );
  });

  it("fills the address and password into every client's config", () => {
    const configs = clientConfigs("http://h:42824/mcp", "secret-1");
    expect(configs.map((config) => config.label)).toEqual([
      "Claude Code",
      "Codex",
      "OpenClaw",
      "Hermes",
      "Other",
    ]);
    for (const config of configs) {
      expect(config.text).toContain("http://h:42824/mcp");
      expect(config.text).toContain("Bearer secret-1");
    }
    const openclaw = JSON.parse(configs[2].text);
    expect(openclaw.mcp.servers.hatchdoor.headers.Authorization).toBe(
      "Bearer secret-1",
    );
  });

  it("uses a placeholder when the password cannot be shown", () => {
    for (const config of clientConfigs("http://h/mcp", null)) {
      expect(config.text).toContain(TOKEN_PLACEHOLDER);
    }
  });
});
