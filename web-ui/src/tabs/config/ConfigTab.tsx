import { useStore } from "../../store/wsStore";
import { apiPost } from "../../lib/api";

export default function ConfigTab() {
  const configPath = useStore((s) => s.config_path);
  const repos = useStore((s) => s.repos);
  const llmModel = useStore((s) => s.llm_model);
  const llmProfile = useStore((s) => s.llm_profile);
  const llmProfiles = useStore((s) => s.llm_profile_options);
  const chatBusy = useStore((s) => s.chat_busy);
  const engineBusy = useStore((s) => s.engine_busy);
  const githubOk = useStore((s) => s.github_ok);
  const llmOk = useStore((s) => s.llm_ok);
  const githubLatency = useStore((s) => s.github_latency_ms);
  const llmLatency = useStore((s) => s.llm_latency_ms);
  const mcpServers = useStore((s) => s.mcp_servers);

  const profileBusy = chatBusy || engineBusy;
  const activeProfile = llmProfile ?? "";

  const onProfileChange = (profile: string) => {
    if (!profile || profile === activeProfile || profileBusy) return;
    void apiPost("/api/config/llm-profile", { profile });
  };

  return (
    <div className="panel">
      <div className="config-section">
        <div className="config-section-title">Config path</div>
        <code className="config-mono">
          {configPath || "(unknown)"}
        </code>
      </div>

      <div className="config-section">
        <div className="config-section-title">Repos</div>
        <div className="ctx-chip-row">
          {repos.length ? (
            repos.map((r) => (
              <span key={r} className="ctx-tool-chip">
                {r}
              </span>
            ))
          ) : (
            <span className="config-muted">none</span>
          )}
        </div>
      </div>

      <div className="config-section">
        <div className="config-section-title">LLM</div>
        {llmProfiles.length > 0 ? (
          <>
            <label className="config-llm-picker">
              <span className="config-muted">Profile</span>
              <select
                className="config-llm-select"
                value={activeProfile}
                disabled={profileBusy}
                onChange={(e) => onProfileChange(e.target.value)}
                title={
                  profileBusy
                    ? "Wait for chat/workflow to finish before switching"
                    : "Switch LLM preset"
                }
              >
                {llmProfiles.map((p) => (
                  <option key={p.id} value={p.id}>
                    {p.id} — {p.model}
                  </option>
                ))}
              </select>
            </label>
            {profileBusy && (
              <div className="config-muted config-llm-hint">
                Switching disabled while agent is busy.
              </div>
            )}
          </>
        ) : (
          <div className="config-muted config-llm-hint">
            Add named endpoints under <code>llm</code> in coworker.yaml to enable
            quick switching.
          </div>
        )}
        <code className="config-mono config-llm-model">
          {llmModel || "(unset)"}
        </code>
      </div>

      <div className="config-section">
        <div className="config-section-title">Connectivity</div>
        <ProbeRow label="GitHub" ok={githubOk} latency={githubLatency} />
        <ProbeRow label="LLM" ok={llmOk} latency={llmLatency} />
        <button
          type="button"
          className="btn btn-primary config-reprobe-btn"
          onClick={() => void apiPost("/api/config/probe")}
        >
          Re-probe
        </button>
      </div>

      <div className="config-section">
        <div className="config-section-title">MCP servers</div>
        {mcpServers.length === 0 ? (
          <div className="config-muted">none configured</div>
        ) : (
          <div>
            {mcpServers.map((s) => (
              <div key={s.id} className="config-mcp-card">
                <div className="ctx-chip-row">
                  <span
                    className={`config-probe-dot ${s.connected ? "ok" : "dead"}`}
                  />
                  <span className="config-mono">{s.id}</span>
                  {s.tool_count > 0 && (
                    <span className="config-muted">{s.tool_count} tools</span>
                  )}
                  {s.last_rpc_ms != null && (
                    <span className="config-muted">{s.last_rpc_ms}ms</span>
                  )}
                </div>
                {s.last_error && (
                  <div className="config-mcp-error">
                    {s.last_error}
                  </div>
                )}
              </div>
            ))}
          </div>
        )}
      </div>

      <div className="config-section">
        <div className="config-section-title">Keyboard shortcuts</div>
        <div className="config-shortcuts">
          <ShortcutRow keys={["Ctrl/⌘", "1–6"]} desc="Switch to tab 1–6" />
          <ShortcutRow keys={["Ctrl/⌘", "K"]} desc="Focus chat input" />
          <ShortcutRow keys={["Enter"]} desc="Insert newline in chat input" />
          <ShortcutRow keys={["Shift", "Enter"]} desc="Send chat message" />
          <ShortcutRow keys={["Esc"]} desc="Cancel generation / close dialog / context drawer" />
        </div>
      </div>
    </div>
  );
}

function ShortcutRow({ keys, desc }: { keys: string[]; desc: string }) {
  return (
    <div className="config-shortcut-row">
      <span className="config-shortcut-keys">
        {keys.map((k, i) => (
          <kbd key={i} className="kbd">
            {k}
          </kbd>
        ))}
      </span>
      <span className="config-shortcut-desc">{desc}</span>
    </div>
  );
}

function ProbeRow({
  label,
  ok,
  latency,
}: {
  label: string;
  ok: boolean;
  latency: number | null;
}) {
  return (
    <div className="config-probe-row">
      <span className={`config-probe-dot ${ok ? "ok" : "dead"}`} />
      <span className="config-probe-label">{label}</span>
      <span className={`config-probe-status${ok ? " ok" : ""}`}>
        {ok ? (latency != null ? `${latency}ms` : "ok") : "offline"}
      </span>
    </div>
  );
}
