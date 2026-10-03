import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";

import { CONTEXTUAL_HELP, ContextualHelpLink } from "../help";
import {
  baseSourceForKind,
  createVault,
  FolderPicker,
  formatWhen,
} from "../settings";
import { fetchRegistryRevision } from "../../vaults";
import type { VaultSummary } from "../../types";
import {
  clientConfigs,
  connectAgent,
  DEMO_URL,
  dismissFirstRun,
  fetchAgentSetup,
  mcpAddress,
  replaceToken,
  setUpdateCheck,
  useFirstRunState,
  type AgentSetup,
  type ClientId,
} from "./firstRun";

/** How often the connect step asks whether an agent has arrived. */
const CONNECT_POLL_MS = 5_000;

type StepId = 1 | 2 | 3 | 4;

/**
 * The first-run checklist (#419): its own page in place of the workspace's
 * empty screen, four steps that tick themselves off from real state.
 */
export function FirstRunChecklist({
  vaults,
  onVaultCreated,
  onAddGitVault,
  onOpenSearch,
}: {
  vaults: VaultSummary[];
  /** A Vault was added here; the workspace should learn about it. */
  onVaultCreated: () => void;
  /** Open Add a Vault, where a Git repository can be chosen. */
  onAddGitVault: () => void;
  onOpenSearch: () => void;
}) {
  const { search } = useFirstRunState();
  const [setup, setSetup] = useState<AgentSetup | null>(null);
  const [setupError, setSetupError] = useState<string | null>(null);
  const [chosen, setChosen] = useState<StepId | null>(null);

  const reload = useCallback(async () => {
    const result = await fetchAgentSetup();
    if (result.ok) {
      setSetup(result.value);
      setSetupError(null);
    } else {
      setSetupError(result.message);
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  const notesDone = vaults.length > 0;
  const agentDone = Boolean(setup?.mcpEnabled && setup.lastAgent);
  const searchDone = search !== null;
  const done: Record<StepId, boolean> = {
    1: notesDone,
    2: true,
    3: agentDone,
    4: searchDone,
  };
  const doneCount = Object.values(done).filter(Boolean).length;
  const allDone = doneCount === 4;
  const next = ([1, 2, 3, 4] as StepId[]).find((step) => !done[step]) ?? null;
  const open = chosen !== null && !done[chosen] ? chosen : next;

  // While agent access is on and nobody has connected, ask again now and
  // then, so the step ticks itself without a reload.
  const waiting = Boolean(setup?.mcpEnabled && !setup.lastAgent);
  useEffect(() => {
    if (!waiting) return;
    const timer = window.setInterval(() => void reload(), CONNECT_POLL_MS);
    return () => window.clearInterval(timer);
  }, [waiting, reload]);

  const stepProps = (id: StepId) => ({
    id,
    done: done[id],
    open: open === id,
    onOpen: () => setChosen(id),
  });

  return (
    <section className="first-run" aria-labelledby="first-run-title">
      <div className="first-run-head">
        <h2 id="first-run-title">
          {allDone ? "You're set up" : "Set up Hatchdoor"}
        </h2>
        <span className="first-run-progress">{doneCount} of 4 done</span>
      </div>
      {allDone ? (
        <p>Your notes are in, your agent can read them, and search works.</p>
      ) : (
        <>
          <p>
            Four steps from nothing to an agent that can search your notes. Each
            one ticks itself off when it is really done.
          </p>
          <p>
            <ContextualHelpLink to={CONTEXTUAL_HELP.noVaults} />
          </p>
        </>
      )}

      <ol className="first-run-steps">
        <Step
          {...stepProps(1)}
          title="Add your notes"
          summary={
            notesDone ? (
              <p>
                {vaults.length === 1 ? (
                  <>
                    Your Vault <strong>{vaults[0].name}</strong> is added.
                  </>
                ) : (
                  <>{vaults.length} Vaults are added.</>
                )}
              </p>
            ) : null
          }
        >
          <AddNotesStep
            onVaultCreated={onVaultCreated}
            onAddGitVault={onAddGitVault}
          />
        </Step>
        <Step
          {...stepProps(2)}
          title="Choose a search model"
          summary={
            <p>Chosen on the screen Hatchdoor showed when it first started.</p>
          }
        />
        <Step
          {...stepProps(3)}
          title="Connect your agent"
          summary={
            agentDone && setup?.lastAgent ? (
              <p>
                <strong>{setup.lastAgent.name}</strong> connected{" "}
                {formatWhen(setup.lastAgent.connected_at) ?? "just now"}.
              </p>
            ) : null
          }
        >
          <ConnectStep
            setup={setup}
            setupError={setupError}
            onSetup={setSetup}
            onRetry={() => void reload()}
          />
        </Step>
        <Step
          {...stepProps(4)}
          title="Try a search"
          summary={
            search ? (
              <p>
                Your search for <strong>{search.query}</strong> found{" "}
                {search.count === 1 ? "1 note" : `${search.count} notes`}, so
                your notes are ready to search.
              </p>
            ) : null
          }
        >
          <p>
            Search for something you know is in your notes. When it finds it,
            Hatchdoor has finished reading them. A large folder can take a few
            minutes the first time.
          </p>
          <div className="first-run-actions">
            <button
              type="button"
              className="settings-btn settings-btn-hot"
              onClick={onOpenSearch}
            >
              Open search
            </button>
          </div>
        </Step>
      </ol>

      <UpdateCheckRow setup={setup} onSetup={setSetup} />

      <div className="first-run-foot">
        {allDone ? (
          <button
            type="button"
            className="settings-btn settings-btn-hot"
            onClick={dismissFirstRun}
          >
            Close the checklist
          </button>
        ) : (
          <button type="button" className="help-link" onClick={dismissFirstRun}>
            Close the checklist
          </button>
        )}
        <span className="first-run-foot-note">
          You can reopen it from Help, under Setup checklist.
        </span>
      </div>
    </section>
  );
}

function Step({
  id,
  title,
  done,
  open,
  onOpen,
  summary,
  children,
}: {
  id: StepId;
  title: string;
  done: boolean;
  open: boolean;
  onOpen: () => void;
  summary?: ReactNode;
  children?: ReactNode;
}) {
  const state = done ? "done" : open ? "open" : "todo";
  return (
    <li className="first-run-step" data-state={state}>
      <div className="first-run-step-head">
        <span className="first-run-mark" aria-hidden="true">
          {done ? "✓" : id}
        </span>
        {done || open ? (
          <h3 className="first-run-step-title">{title}</h3>
        ) : (
          <h3 className="first-run-step-title">
            <button
              type="button"
              className="first-run-step-open"
              onClick={onOpen}
            >
              {title}
            </button>
          </h3>
        )}
        <span className="first-run-step-status">{done ? "Done" : ""}</span>
      </div>
      {done && summary ? (
        <div className="first-run-step-summary">{summary}</div>
      ) : null}
      {!done && open && children ? (
        <div className="first-run-step-body">{children}</div>
      ) : null}
    </li>
  );
}

function lastSegment(path: string): string {
  const parts = path.split("/").filter(Boolean);
  return parts.at(-1) ?? "";
}

function AddNotesStep({
  onVaultCreated,
  onAddGitVault,
}: {
  onVaultCreated: () => void;
  onAddGitVault: () => void;
}) {
  const [picking, setPicking] = useState(false);
  const [path, setPath] = useState("");
  const [name, setName] = useState("");
  const nameTouched = useRef(false);
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  if (!picking) {
    return (
      <>
        <p>
          Point Hatchdoor at the folder that holds your Markdown notes. It reads
          them where they are and does not move or copy them.
        </p>
        <div className="first-run-actions">
          <button
            type="button"
            className="settings-btn settings-btn-hot"
            onClick={() => setPicking(true)}
          >
            Pick a folder
          </button>
          <button
            type="button"
            className="settings-btn"
            onClick={onAddGitVault}
          >
            Use a Git repository instead
          </button>
        </div>
        <p className="first-run-demo-offer">
          Want to look around first?{" "}
          <a
            className="help-link"
            href={DEMO_URL}
            target="_blank"
            rel="noopener noreferrer"
          >
            Open the public demo
          </a>
          , which has example Vaults to read and search. It opens in a new tab
          and changes nothing here.
        </p>
      </>
    );
  }

  const submit = async () => {
    if (submitting) return;
    setError(null);
    const trimmed = name.trim();
    if (!path) {
      setError("Pick the folder that holds your notes.");
      return;
    }
    if (!trimmed) {
      setError("Give these notes a name.");
      return;
    }
    setSubmitting(true);
    const revision = await fetchRegistryRevision();
    if (revision === null) {
      setSubmitting(false);
      setError(
        "Could not reach the server. Check the connection and try again.",
      );
      return;
    }
    const result = await createVault({
      expectedRegistryRevision: revision,
      name: trimmed,
      source: baseSourceForKind("own", path),
      excludePatterns: [],
    });
    setSubmitting(false);
    if (!result.ok) {
      setError(
        result.code === "registry_revision_conflict"
          ? "This changed elsewhere just now. Try again."
          : (result.message ?? "These notes could not be added."),
      );
      return;
    }
    onVaultCreated();
  };

  return (
    <>
      <label className="settings-row first-run-name">
        <span>
          <span className="settings-row-label">Name</span>
          <span className="settings-row-help">
            What Hatchdoor calls these notes.
          </span>
        </span>
        <input
          className="settings-input"
          aria-label="Name"
          value={name}
          onChange={(event) => {
            nameTouched.current = true;
            setName(event.target.value);
          }}
        />
      </label>
      <div className="folder-picker-field">
        <span>
          <span className="settings-row-label">Folder</span>
          <span className="settings-row-help">
            The folders Hatchdoor can see, with how many notes each holds.
          </span>
        </span>
        <FolderPicker
          value={path}
          onPick={(picked) => {
            setPath(picked);
            if (!nameTouched.current) setName(lastSegment(picked));
          }}
        />
      </div>
      {error ? <ErrorNotice message={error} /> : null}
      <div className="first-run-actions">
        <button
          type="button"
          className="settings-btn settings-btn-hot"
          disabled={submitting}
          onClick={() => void submit()}
        >
          {submitting ? "Adding…" : "Add these notes"}
        </button>
        <button
          type="button"
          className="settings-btn"
          disabled={submitting}
          onClick={() => {
            setPicking(false);
            setError(null);
          }}
        >
          Cancel
        </button>
      </div>
    </>
  );
}

function ConnectStep({
  setup,
  setupError,
  onSetup,
  onRetry,
}: {
  setup: AgentSetup | null;
  setupError: string | null;
  onSetup: (setup: AgentSetup) => void;
  onRetry: () => void;
}) {
  // The password, only in this view and only right after it was made: the
  // server never shows it again.
  const [token, setToken] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [client, setClient] = useState<ClientId>("claude");
  const [copied, setCopied] = useState(false);

  if (!setup) {
    return setupError ? (
      <div className="settings-notice settings-notice-err" role="alert">
        <p>{setupError}</p>
        <button type="button" className="settings-btn" onClick={onRetry}>
          Try again
        </button>
      </div>
    ) : (
      <p className="first-run-muted">Loading…</p>
    );
  }

  const run = async (action: () => ReturnType<typeof connectAgent>) => {
    setBusy(true);
    setError(null);
    const result = await action();
    setBusy(false);
    if (!result.ok) {
      setError(result.message);
      return;
    }
    setToken(result.value.token);
    setCopied(false);
    onSetup(result.value.setup);
  };

  const errorLine = error ? <ErrorNotice message={error} /> : null;

  if (!setup.mcpEnabled) {
    return (
      <>
        <p>
          Lets an AI agent such as Claude Code or Codex search and read your
          notes. One click turns agent access on, read-only, makes the agent's
          password, and shows the settings to paste into your agent.
        </p>
        {errorLine}
        <div className="first-run-actions">
          <button
            type="button"
            className="settings-btn settings-btn-hot"
            disabled={busy}
            onClick={() => void run(() => connectAgent(setup))}
          >
            {busy ? "Connecting…" : "Connect an agent"}
          </button>
          <ContextualHelpLink to={CONTEXTUAL_HELP.agentSettings} />
        </div>
        <WritesLine setup={setup} />
      </>
    );
  }

  const configs = clientConfigs(
    mcpAddress(setup.publicUrl, window.location.origin),
    token,
  );
  const config = configs.find((item) => item.id === client) ?? configs[0];
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(config.text);
      setCopied(true);
    } catch {
      setCopied(false);
    }
  };

  return (
    <>
      {token ? (
        <div className="settings-notice" role="note">
          <strong>Copy this now.</strong> The password inside it is shown only
          this once. If you lose it, make a new one; the old one then stops
          working.
        </div>
      ) : setup.tokenLocked ? (
        <p>
          Agent access is on. Its password is set in the server's configuration
          file as <code>HATCHDOOR_MCP_BEARER_TOKEN</code>; put it where the
          config below says <code>&lt;your MCP password&gt;</code>.
        </p>
      ) : (
        <p>
          Agent access is on. The password was shown once, when it was made. If
          you no longer have it, make a new one; agents using the old one then
          stop getting in.
        </p>
      )}
      {errorLine}
      <div
        className="settings-segmented first-run-clients"
        role="group"
        aria-label="Your agent"
      >
        {configs.map((item) => (
          <button
            key={item.id}
            type="button"
            aria-pressed={item.id === config.id}
            onClick={() => {
              setClient(item.id);
              setCopied(false);
            }}
          >
            {item.label}
          </button>
        ))}
      </div>
      <div className="first-run-config">
        <p className="settings-row-help">{config.where}</p>
        <pre className="first-run-code">
          <code>{config.text}</code>
        </pre>
        <button
          type="button"
          className="settings-btn"
          onClick={() => void copy()}
        >
          {copied ? "Copied" : "Copy"}
        </button>
      </div>
      <p className="first-run-wait" role="status">
        <span className="first-run-pulse" aria-hidden="true" />
        Waiting for your agent to connect. This ticks itself once it makes its
        first request.
      </p>
      {setup.tokenLocked ? null : (
        <p>
          <button
            type="button"
            className="help-link"
            disabled={busy}
            onClick={() => void run(replaceToken)}
          >
            Make a new password
          </button>
        </p>
      )}
      <WritesLine setup={setup} />
    </>
  );
}

function ErrorNotice({ message }: { message: string }) {
  return (
    <p className="settings-notice settings-notice-err" role="alert">
      {message}
    </p>
  );
}

function writesSentence(setup: AgentSetup): string {
  if (setup.writesLocked && setup.writesEnabled) {
    return "The server's configuration file lets agents change notes as well as read them, so this cannot be turned off here.";
  }
  if (setup.mcpEnabled && setup.writesEnabled) {
    return "Your agent can change notes as well as read them: Let assistants change notes is on in Settings, under Agent access (MCP).";
  }
  return "Your agent starts read-only: it can search and read, but not create, edit or delete. To let it change notes, turn on Let assistants change notes in Settings, under Agent access (MCP).";
}

function WritesLine({ setup }: { setup: AgentSetup }) {
  return (
    <div className="first-run-writes">
      <p className="settings-row-label">Changing notes is a separate switch</p>
      <p>
        {writesSentence(setup)}{" "}
        <ContextualHelpLink to={CONTEXTUAL_HELP.agentWrites} />
      </p>
    </div>
  );
}

function UpdateCheckRow({
  setup,
  onSetup,
}: {
  setup: AgentSetup | null;
  onSetup: (setup: AgentSetup) => void;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  if (!setup) return null;
  const on = setup.updateCheckEnabled;
  const toggle = async () => {
    setBusy(true);
    setError(null);
    const result = await setUpdateCheck(!on);
    setBusy(false);
    if (result.ok) onSetup(result.value);
    else setError(result.message);
  };
  return (
    <div className="first-run-update">
      <div className="first-run-update-head">
        <span className="settings-row-label">
          Optional: tell me when there is a new version
        </span>
        {setup.updateCheckLocked ? (
          <span className="first-run-muted">
            {on ? "On" : "Off"}, set in the server's configuration file
          </span>
        ) : (
          <button
            type="button"
            className="settings-toggle"
            aria-label="Tell me when there is a new version"
            aria-pressed={on}
            disabled={busy}
            onClick={() => void toggle()}
          >
            <span className="settings-toggle-track">
              <span className="settings-toggle-knob" />
            </span>
            <span>{on ? "On" : "Off"}</span>
          </button>
        )}
      </div>
      <p>
        Once a day, Hatchdoor sends one request to GitHub's public list of
        Hatchdoor releases, carrying this server's IP address and the user-agent
        Hatchdoor, nothing else. A banner says when a newer version is out;
        Hatchdoor never updates itself.{" "}
        <ContextualHelpLink to={CONTEXTUAL_HELP.updateCheck} />
      </p>
      {error ? <ErrorNotice message={error} /> : null}
    </div>
  );
}
