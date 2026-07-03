import { useDeferredValue } from "react";
import { useStore } from "../../store/wsStore";
import Markdown from "../../components/Markdown";
import ReasoningCard from "../../components/ReasoningCard";
import { splitStreaming } from "./streamSplit";
import { toolMeta } from "./parser";
import { BookOpen, Sparkles, Zap } from "lucide-react";

/** Raw live fields — use for mount/visibility decisions (no deferred lag). */
function useLiveVisibility() {
  const chatBusy = useStore((s) => s.chat_busy);
  const streaming = useStore((s) => s.chat_streaming);
  const reasoning = useStore((s) => s.chat_reasoning);
  const toolRunning = useStore((s) => s.chat_tool_running);
  const toolRunningDetail = useStore((s) => s.chat_tool_running_detail);
  const toolPending = useStore((s) => s.chat_tool_pending);
  const compressing = useStore((s) => s.chat_reasoning_compressing);
  const activityFlow = useStore((s) => s.chat_activity_flow);
  return {
    chatBusy,
    streaming,
    reasoning,
    toolRunning,
    toolRunningDetail,
    toolPending,
    compressing,
    activityFlow,
  };
}

/** Deferred streaming/reasoning for smoother in-turn rendering only. */
function useLiveDisplayState() {
  const raw = useLiveVisibility();
  const streaming = useDeferredValue(raw.streaming);
  const reasoning = useDeferredValue(raw.reasoning);
  return { ...raw, streaming, reasoning };
}

export function useLiveZoneActive() {
  // Live chrome only while a chat turn is in flight — avoids stale deferred
  // streaming/reasoning keeping the zone mounted after chat_busy goes false.
  return useStore((s) => s.chat_busy);
}

export default function LiveZone() {
  const chatBusy = useStore((s) => s.chat_busy);
  const {
    streaming,
    reasoning,
    toolRunning,
    toolRunningDetail,
    toolPending,
    compressing,
    activityFlow,
  } = useLiveDisplayState();

  const mcpServers = useStore((s) => s.mcp_servers);
  if (!chatBusy) return null;
  const mcpPrefixes = mcpServers.map((s) => ({
    id: s.id,
    prefix: s.prefix || `${s.id}_`,
  }));
  const runningName = toolRunning || toolPending || "";
  const runningMeta = runningName ? toolMeta(runningName, mcpPrefixes) : null;

  return (
    <div className="live-zone has-activity" aria-live="polite">
      <div className="activity-stack">
        {/* Running tool */}
        {(toolRunning || toolPending) && (
          <div className="tool-card status-running live-tool is-collapsed">
            <div className="tool-card-header tool-card-header-static">
              <span className="tool-card-icon" aria-hidden="true">
                {runningMeta?.icon ?? (toolRunning ? "→" : "⏳")}
              </span>
              <div className="tool-card-title-wrap">
                <div className="tool-card-title-row">
                  <span className="tool-card-title">
                    {runningMeta?.label ?? runningName}
                  </span>
                  {runningMeta && runningMeta.label !== runningName && (
                    <span className="tool-card-fn">{runningName}</span>
                  )}
                </div>
                {toolRunningDetail && (
                  <span className="tool-card-arg-line">{toolRunningDetail}</span>
                )}
              </div>
              <div className="tool-card-trail">
                <span className="tool-status-pill status-running">
                  {toolRunning ? "Running" : "Queued"}
                </span>
                <span className="tool-spinner" aria-hidden="true" />
              </div>
            </div>
          </div>
        )}

        {/* Activity flow */}
        {activityFlow && (
          <div className="activity-flow">
            <div className="activity-flow-head">
              <span className="activity-icon">
                {activityFlow.kind === "Skill" ? <BookOpen size={14} aria-hidden="true" /> : <Zap size={14} aria-hidden="true" />}
              </span>
              <span className="activity-title">
                {activityFlow.kind === "Skill" ? "Skill" : "Activity"}
              </span>
            </div>
            <div className="activity-flow-body">{activityFlow.text}</div>
          </div>
        )}

        {/* Summarizing */}
        {compressing && (
          <div className="activity-thinking activity-summarizing">
            <span className="tool-spinner" />
            <span className="activity-title">Summarizing context…</span>
          </div>
        )}

        {/* Reasoning (live) — hidden once assistant streaming starts (legacy). */}
        {reasoning && !streaming && <ReasoningCard text={reasoning} live />}

        {/* Streaming reply — split into a stable Markdown prefix and an
            unstable plain-text tail so partial formatting is visible without
            the jitter from unclosed code fences / tables re-parsing on every
            token. The cursor lives at the end of the unstable tail. */}
        {streaming && <StreamingReply text={streaming} />}

        {/* Thinking (no other activity) */}
        {!streaming &&
          !reasoning &&
          !toolRunning &&
          !toolPending &&
          !compressing &&
          !activityFlow && (
            <div className="activity-thinking">
              <span className="tool-spinner" />
              <span className="activity-title">Thinking…</span>
            </div>
          )}
      </div>
    </div>
  );
}

/** Streaming reply: stable Markdown prefix + unstable plain-text tail. The
 * split prevents unclosed code fences / tables from swallowing the freshly
 * typed text and avoids re-parsing the whole stream on every token (jitter). */
function StreamingReply({ text }: { text: string }) {
  const { stable, unstable } = splitStreaming(text);
  return (
    <div className="activity-streaming">
      <div className="activity-streaming-head">
        <span className="activity-icon"><Sparkles size={14} aria-hidden="true" /></span>
        <span className="activity-title">Assistant</span>
      </div>
      <div className="activity-streaming-body is-streaming" aria-hidden="true">
        {stable && <Markdown>{stable}</Markdown>}
        {unstable && (
          <div className="streaming-tail">
            {stable && <span className="streaming-tail-br" />}
            <span className="streaming-plain">{unstable}</span>
            <span className="reasoning-cursor" aria-hidden="true" />
          </div>
        )}
        {!unstable && <span className="reasoning-cursor" aria-hidden="true" />}
      </div>
    </div>
  );
}
