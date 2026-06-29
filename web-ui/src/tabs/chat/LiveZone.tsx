import { useDeferredValue } from "react";
import { useStore } from "../../store/wsStore";
import Markdown from "../../components/Markdown";
import ReasoningCard from "../../components/ReasoningCard";
import { splitStreaming } from "./streamSplit";
import { ArrowRight, BookOpen, Clock, Sparkles, Zap } from "lucide-react";

interface LiveState {
  chatBusy: boolean;
  streaming: string | null;
  reasoning: string | null;
  toolRunning: string | null;
  toolRunningDetail: string | null;
  toolPending: string | null;
  compressing: boolean;
  activityFlow: { kind: string; text: string } | null;
}

/** Single store subscription for all live-zone fields. Both useLiveZoneActive
 * and LiveZone call this, so there's only one selector evaluation per render
 * instead of 15 separate useStore calls. */
function useLiveState(): LiveState {
  const streamingRaw = useStore((s) => s.chat_streaming);
  const reasoningRaw = useStore((s) => s.chat_reasoning);
  const streaming = useDeferredValue(streamingRaw);
  const reasoning = useDeferredValue(reasoningRaw);
  const chatBusy = useStore((s) => s.chat_busy);
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

export function useLiveZoneActive() {
  const s = useLiveState();
  return Boolean(
    s.chatBusy ||
      s.streaming ||
      s.reasoning ||
      s.toolRunning ||
      s.toolPending ||
      s.compressing ||
      s.activityFlow,
  );
}

export default function LiveZone() {
  const {
    chatBusy,
    streaming,
    reasoning,
    toolRunning,
    toolRunningDetail,
    toolPending,
    compressing,
    activityFlow,
  } = useLiveState();

  const hasAnything =
    chatBusy ||
    streaming ||
    reasoning ||
    toolRunning ||
    toolPending ||
    compressing ||
    activityFlow;

  if (!hasAnything) return null;

  return (
    <div className="live-zone has-activity" aria-live="polite">
      <div className="activity-stack">
        {/* Running tool */}
        {(toolRunning || toolPending) && (
          <div className={`tool-card status-running live-tool is-collapsed`}>
            <div className="tool-card-header">
              <span className="tool-card-icon">
                {toolRunning ? <ArrowRight size={14} aria-hidden="true" /> : <Clock size={14} aria-hidden="true" />}
              </span>
              <div className="tool-card-title-wrap">
                <span className="tool-card-title">
                  {toolRunning || toolPending}
                </span>
                {toolRunningDetail && (
                  <span className="tool-card-fn">{toolRunningDetail}</span>
                )}
              </div>
              <span className="tool-spinner" />
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
        {chatBusy &&
          !streaming &&
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
