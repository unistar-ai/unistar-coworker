import { useStore } from "../../store/wsStore";
import { apiPost } from "../../lib/api";

export default function ConfigTab() {
  const configPath = useStore((s) => s.config_path);
  const repos = useStore((s) => s.repos);
  const llmModel = useStore((s) => s.llm_model);
  const githubOk = useStore((s) => s.github_ok);
  const llmOk = useStore((s) => s.llm_ok);
  const githubLatency = useStore((s) => s.github_latency_ms);
  const llmLatency = useStore((s) => s.llm_latency_ms);
  const mcpServers = useStore((s) => s.mcp_servers);

  return (
    <div className="panel">
      <div className="config-section">
        <div className="config-section-title">Config path</div>
        <code style={{ fontFamily: "var(--font-mono)", fontSize: "12px" }}>
          {configPath || "(unknown)"}
        </code>
      </div>

      <div className="config-section">
        <div className="config-section-title">Repos</div>
        <div className="flex flex-wrap gap-1">
          {repos.length ? (
            repos.map((r) => (
              <span key={r} className="ctx-tool-chip">
                {r}
              </span>
            ))
          ) : (
            <span className="text-text-muted">none</span>
          )}
        </div>
      </div>

      <div className="config-section">
        <div className="config-section-title">LLM model</div>
        <code style={{ fontFamily: "var(--font-mono)", fontSize: "12px" }}>
          {llmModel || "(unset)"}
        </code>
      </div>

      <div className="config-section">
        <div className="config-section-title">Connectivity</div>
        <ProbeRow label="GitHub" ok={githubOk} latency={githubLatency} />
        <ProbeRow label="LLM" ok={llmOk} latency={llmLatency} />
        <button
          type="button"
          className="btn btn-primary"
          style={{ marginTop: "0.5rem" }}
          onClick={() => void apiPost("/api/config/probe")}
        >
          Re-probe
        </button>
      </div>

      <div className="config-section">
        <div className="config-section-title">MCP servers</div>
        {mcpServers.length === 0 ? (
          <div className="text-text-muted">none configured</div>
        ) : (
          <div>
            {mcpServers.map((s) => (
              <div key={s.id} className="config-mcp-card">
                <div className="flex items-center gap-2">
                  <span
                    className={`config-probe-dot ${s.connected ? "ok" : "dead"}`}
                  />
                  <span style={{ fontFamily: "var(--font-mono)" }}>{s.id}</span>
                  {s.tool_count > 0 && (
                    <span className="text-text-muted">{s.tool_count} tools</span>
                  )}
                  {s.last_rpc_ms != null && (
                    <span className="text-text-muted">{s.last_rpc_ms}ms</span>
                  )}
                </div>
                {s.last_error && (
                  <div style={{ color: "var(--danger)", marginTop: "0.25rem" }}>
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
          <ShortcutRow keys={["Enter"]} desc="Send chat message" />
          <ShortcutRow keys={["Shift", "Enter"]} desc="Insert newline" />
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
      <span style={{ width: "4rem" }}>{label}</span>
      <span style={{ color: ok ? "var(--ok)" : "var(--danger)" }}>
        {ok ? (latency != null ? `${latency}ms` : "ok") : "offline"}
      </span>
    </div>
  );
}
