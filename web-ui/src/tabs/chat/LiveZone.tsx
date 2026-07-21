import { useDeferredValue, useMemo } from "react";
import { useStore } from "../../store/wsStore";
import Markdown from "../../components/Markdown";
import { splitStreaming } from "./streamSplit";
import MessageTurnFrame from "./MessageTurnFrame";
import { useTrailingAnswerRelease } from "./useTrailingAnswerRelease";
import {
  BookOpen,
  MessageSquare,
  Wrench,
  Zap,
} from "lucide-react";
import { partsProcessStats } from "./messageParts";
import { partsToDisplaySteps } from "./partDisplay";
import {
  decorateLiveDisplaySteps,
  isLiveProcessActive,
  liveHasProcessPanel,
  resolveLiveProcessParts,
  type LiveTransportState,
} from "./liveParts";
import { resolveLiveHeaderCandidate, useStableLiveHeader } from "./liveHeader";
import TurnProcessPanel from "./TurnProcessPanel";
import { BlockRenderer } from "./ChatHistory";
import { useLiveTurnElapsed } from "./useLiveTurnElapsed";

function useLiveVisibility(): LiveTransportState & { chatBusy: boolean } {
  const chatBusy = useStore((s) => s.chat_busy);
  const streaming = useStore((s) => s.chat_streaming);
  const reasoning = useStore((s) => s.chat_reasoning);
  const toolRunning = useStore((s) => s.chat_tool_running);
  const toolRunningDetail = useStore((s) => s.chat_tool_running_detail);
  const toolPending = useStore((s) => s.chat_tool_pending);
  const compressing = useStore((s) => s.chat_reasoning_compressing);
  const activityFlow = useStore((s) => s.chat_activity_flow);
  const turnPhase = useStore((s) => s.chat_turn_phase);
  return {
    chatBusy,
    streaming,
    reasoning,
    toolRunning,
    toolRunningDetail,
    toolPending,
    compressing,
    activityFlow,
    turnPhase,
  };
}

function useLiveDisplayState() {
  const raw = useLiveVisibility();
  const streaming = useDeferredValue(raw.streaming);
  const reasoning = useDeferredValue(raw.reasoning);
  return { ...raw, streaming, reasoning };
}

function LiveExtraPreview({
  live,
  showHeldAnswer,
  streaming,
}: {
  live: LiveTransportState;
  showHeldAnswer: boolean;
  streaming: string | null;
}) {
  return (
    <>
      {live.activityFlow && (
        <div className="chat-process-preview-row kind-tool is-live">
          <span className="chat-process-preview-icon" aria-hidden="true">
            {live.activityFlow.kind === "Skill" ? (
              <BookOpen size={15} strokeWidth={2} />
            ) : (
              <Zap size={15} strokeWidth={2} />
            )}
          </span>
          <span className="chat-process-preview-text">
            <span className="chat-process-preview-title">
              {live.activityFlow.kind === "Skill" ? "技能" : "活动"}
            </span>
            <span className="chat-process-preview-subtitle">{live.activityFlow.text}</span>
          </span>
        </div>
      )}
      {live.compressing && (
        <div className="chat-process-preview-row kind-thought is-live">
          <span className="chat-process-preview-icon" aria-hidden="true">
            <Wrench size={15} strokeWidth={2} />
          </span>
          <span className="chat-process-preview-title">压缩上下文</span>
          <span className="tool-spinner" aria-hidden="true" />
        </div>
      )}
      {showHeldAnswer && streaming && (
        <div className="chat-process-preview-row kind-answer is-live">
          <span className="chat-process-preview-icon" aria-hidden="true">
            <MessageSquare size={15} strokeWidth={2} />
          </span>
          <span className="chat-process-preview-text">
            <span className="chat-process-preview-title">生成回复</span>
            <span className="chat-process-preview-subtitle reasoning-live">
              {streaming.split("\n").slice(-2).join("\n")}
            </span>
          </span>
          <span className="chat-process-preview-status status-running">
            <span className="tool-spinner" aria-hidden="true" />
            等待释放
          </span>
        </div>
      )}
    </>
  );
}

export default function LiveZone() {
  const chatLines = useStore((s) => s.chat_lines);
  const outputs = useStore((s) => s.chat_tool_outputs);
  const reasoningOriginals = useStore((s) => s.chat_reasoning_originals);
  const wsTurnParts = useStore((s) => s.chat_turn_parts);
  const mcpServers = useStore((s) => s.mcp_servers);
  const live = useLiveDisplayState();
  const { chatBusy, streaming, reasoning, toolRunning, toolPending, compressing, activityFlow } =
    live;
  const elapsedMs = useLiveTurnElapsed(chatBusy);

  const mcpPrefixes = useMemo(
    () => mcpServers.map((s) => ({ id: s.id, prefix: s.prefix || `${s.id}_` })),
    [mcpServers],
  );

  const processParts = useMemo(
    () => resolveLiveProcessParts(wsTurnParts, chatLines, outputs, reasoningOriginals, live),
    [wsTurnParts, chatLines, outputs, reasoningOriginals, live],
  );

  const steps = useMemo(() => {
    const base = partsToDisplaySteps(processParts, mcpPrefixes);
    return decorateLiveDisplaySteps(base, live);
  }, [processParts, mcpPrefixes, live]);

  const stats = useMemo(() => partsProcessStats(processParts), [processParts]);
  const hasActiveProcess = isLiveProcessActive(processParts, live);
  const headerCandidate = useMemo(
    () =>
      resolveLiveHeaderCandidate(
        processParts,
        live,
        elapsedMs,
        mcpPrefixes,
        !hasActiveProcess,
      ),
    [processParts, live, elapsedMs, mcpPrefixes, hasActiveProcess],
  );
  const stableHeader = useStableLiveHeader(headerCandidate, hasActiveProcess);
  const hasProcess = liveHasProcessPanel(processParts, live);

  const showStreamingAnswer = useTrailingAnswerRelease(hasActiveProcess, streaming, chatBusy);
  const thinkingOnly =
    !streaming &&
    !reasoning &&
    !toolRunning &&
    !toolPending &&
    !compressing &&
    !activityFlow &&
    !hasActiveProcess;

  if (!chatBusy) return null;

  const renderBlock = (props: Parameters<typeof BlockRenderer>[0]) => (
    <BlockRenderer {...props} />
  );

  return (
    <article className="chat-agent-turn live-zone" aria-live="polite" aria-busy={chatBusy || undefined}>
      <MessageTurnFrame role="agent" name="助手">
        {hasProcess && (
          <TurnProcessPanel
            steps={steps}
            summary={stableHeader.text}
            summaryShimmer={stableHeader.shimmer}
            stats={stats}
            mcpPrefixes={mcpPrefixes}
            renderBlock={renderBlock}
            defaultCollapsed
            variant="live"
            isLiveProgress={hasActiveProcess || chatBusy}
            extraExpanded={
              <LiveExtraPreview
                live={live}
                showHeldAnswer={!showStreamingAnswer}
                streaming={streaming}
              />
            }
          />
        )}

        {showStreamingAnswer && streaming && <StreamingReply text={streaming} />}

        {thinkingOnly && (
          <div className="chat-live-thinking activity-thinking">
            <span className="tool-spinner" aria-hidden="true" />
            <span className="activity-title">思考中…</span>
          </div>
        )}
      </MessageTurnFrame>
    </article>
  );
}

function StreamingReply({ text }: { text: string }) {
  const { stable, unstable } = splitStreaming(text);
  return (
    <div className="message-turn-body-inner chat-streaming-reply">
      {stable && (
        <Markdown variant="turn" streaming={false}>
          {stable}
        </Markdown>
      )}
      {unstable ? (
        <div className="streaming-tail">
          {stable && <span className="streaming-tail-br" />}
          <span className="chat-turn-plain streaming-plain">{unstable}</span>
          <span className="reasoning-cursor" aria-hidden="true" />
        </div>
      ) : (
        stable && <span className="reasoning-cursor" aria-hidden="true" />
      )}
    </div>
  );
}
