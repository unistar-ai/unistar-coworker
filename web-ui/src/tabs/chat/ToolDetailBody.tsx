import { ExternalLink } from "lucide-react";
import type { MouseEvent } from "react";
import { useChatUiStore } from "../../store/chatUiStore";
import { toolMeta, type ToolGroup, type ToolStep } from "./parser";
import { buildContextToolFocus } from "./contextFocus";
import {
  preferArgBlock,
  resolveToolArgPairs,
} from "./toolDisplay";
import { ToolStepOutput } from "./toolOutput";
import ToolMarkdownToggle from "./ToolMarkdownToggle";

function contentSteps(group: ToolGroup): ToolStep[] {
  return group.steps.filter((s) => {
    if (s.kind === "start") return false;
    if (s.kind === "done") return Boolean(s.output);
    return true;
  });
}

/** Shared tool body — process panel (`process`) vs standalone cards (`card`). */
export default function ToolDetailBody({
  group,
  mcpPrefixes,
  variant = "card",
  showContextLink = true,
}: {
  group: ToolGroup;
  mcpPrefixes: { id: string; prefix: string }[];
  /** `process` = inside TurnProcessPanel expand (two-pane args/result). */
  variant?: "process" | "card";
  showContextLink?: boolean;
}) {
  const openContextForTool = useChatUiStore((s) => s.openContextForTool);
  const isProcess = variant === "process";
  const meta = toolMeta(group.toolName, mcpPrefixes);
  const argPairs = resolveToolArgPairs(group);
  const blockArgs = argPairs.filter((p) => preferArgBlock(p.key, p.value));
  const listArgs = argPairs.filter((p) => !preferArgBlock(p.key, p.value));
  const hasArgs = argPairs.length > 0;
  const steps = contentSteps(group);
  const running = group.status === "running" || group.status === "pending";
  const hasResult = steps.length > 0;

  const openContext = (e: MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    openContextForTool(buildContextToolFocus(group));
  };

  return (
    <div
      className={`tool-inline-detail status-${group.status}${
        isProcess ? " is-process" : " is-card-body"
      }`}
    >
      {!isProcess && (meta.source || showContextLink) && (
        <div className="tool-inline-meta">
          {meta.source && (
            <span className="tool-source-chip" title={`工具后端: ${meta.source.source}`}>
              {meta.source.source}
            </span>
          )}
          {showContextLink && (
            <button type="button" className="tool-inline-context-link" onClick={openContext}>
              在上下文中查看
            </button>
          )}
        </div>
      )}

      {hasArgs && (
        <section className="tool-detail-pane is-args" aria-label="参数">
          <header className="tool-detail-pane-head">
            <span className="tool-detail-pane-label">参数</span>
          </header>
          <div className="tool-detail-pane-body">
            {blockArgs.map((p) => (
              <div key={p.key} className="tool-inline-command">
                <div className="tool-inline-command-label">{p.key}</div>
                <pre className="tool-inline-command-body">{p.value}</pre>
              </div>
            ))}
            {listArgs.length > 0 && (
              <dl className="tool-arg-list">
                {listArgs.map((p, i) => (
                  <div key={`${p.key}-${i}`} className="tool-arg-list-row">
                    <dt className="tool-arg-list-k">{p.key}</dt>
                    <dd className="tool-arg-list-v">{p.value || "—"}</dd>
                  </div>
                ))}
              </dl>
            )}
          </div>
        </section>
      )}

      <section className="tool-detail-pane is-result" aria-label="结果">
        <header className="tool-detail-pane-head">
          <span className="tool-detail-pane-label">结果</span>
          {group.ms != null && group.ms !== "" && (
            <span className="tool-detail-pane-meta">{group.ms}ms</span>
          )}
          <ToolMarkdownToggle className="tool-detail-pane-action" />
        </header>
        <div className="tool-detail-pane-body">
          {hasResult ? (
            <div className="tool-inline-body">
              {steps.map((s, i) => (
                <ToolStepBody
                  key={i}
                  step={s}
                  toolName={group.toolName}
                  inline={isProcess}
                />
              ))}
            </div>
          ) : running ? (
            <div className="tool-process-pending">
              <span className="tool-spinner" aria-hidden="true" />
              <span>正在执行…</span>
            </div>
          ) : (
            <div className="tool-inline-empty">无输出</div>
          )}
        </div>
      </section>

      {isProcess && showContextLink && (
        <button type="button" className="tool-process-context-link" onClick={openContext}>
          <ExternalLink size={12} strokeWidth={2} aria-hidden="true" />
          在上下文中查看
        </button>
      )}
    </div>
  );
}

function ToolStepBody({
  step,
  toolName,
  inline,
}: {
  step: ToolStep;
  toolName: string;
  inline: boolean;
}) {
  if (step.kind === "done" && step.output) {
    return (
      <ToolStepOutput
        output={step.output}
        outputKey={`step-${step.index}`}
        toolName={toolName ?? step.name ?? undefined}
        inline={inline}
      />
    );
  }
  if (step.output || step.text) {
    return (
      <ToolStepOutput
        output={step.output || step.text}
        outputKey={`step-${step.index}`}
        toolName={toolName}
        inline={inline}
      />
    );
  }
  return null;
}
