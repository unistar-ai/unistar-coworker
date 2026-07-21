import { useMemo, useState, createContext, useContext } from "react";
import {
  Check,
  Copy,
  RefreshCw,
} from "lucide-react";
import MessageTurnFrame, { UserBubbleFrame } from "./MessageTurnFrame";
import Markdown from "../../components/Markdown";
import { useChatUiStore } from "../../store/chatUiStore";
import { useStore } from "../../store/wsStore";
import { apiPost } from "../../lib/api";
import {
  countFailedToolGroups,
  formatTurnProcessSummary,
  turnProcessDurationMs,
} from "./parser";
import type { AgentTurn, ChatBlock, ChatMessage } from "./parser";
import {
  partsProcessStats,
  splitTurnParts,
  resolveHistoryAgentParts,
} from "./messageParts";
import { partsToDisplaySteps } from "./partDisplay";
import TurnProcessPanel from "./TurnProcessPanel";
import { highlightSearchText } from "./searchHighlight";
import ErrorMessageBody from "./ErrorMessageBody";
import AssistantTurnMeta from "./AssistantTurnMeta";
import { turnMetaFromAgentTurn } from "./turnStats";

export const ChatSearchQueryContext = createContext("");

function useLineTimeIso(lineIndex: number): string | undefined {
  const iso = useStore((s) => s.chat_line_times[String(lineIndex)]);
  return iso || undefined;
}

export interface BlockRendererProps {
  block: ChatBlock;
  mcpPrefixes: { id: string; prefix: string }[];
  compact?: boolean;
  hideLabel?: boolean;
  /** Minimal embed inside process step detail (no nested cards). */
  inline?: boolean;
}

/** Readable turn body — markdown or plain text with consistent typography. */
export function TurnMessageBody({
  msg,
  nested = false,
  framed = false,
}: {
  msg: ChatMessage;
  /** Inside process panel — no extra left indent. */
  nested?: boolean;
  /** Inside MessageTurnFrame — no legacy left padding. */
  framed?: boolean;
}) {
  const searchQuery = useContext(ChatSearchQueryContext);
  const plain = searchQuery.trim()
    ? highlightSearchText(msg.body, searchQuery)
    : msg.body;
  const inner = msg.md ? (
    <Markdown variant="turn" highlightQuery={searchQuery || undefined}>
      {msg.body}
    </Markdown>
  ) : (
    <div className="chat-turn-plain">{plain}</div>
  );
  if (nested) {
    return <div className="chat-process-inline-body">{inner}</div>;
  }
  if (framed) {
    return <div className="message-turn-body-inner">{inner}</div>;
  }
  return <div className="chat-turn-content">{inner}</div>;
}

/** User turn — plain (Cherry MessageHeader) or bubble layout. */
export function UserTurnView({ msg }: { msg: ChatMessage }) {
  const userMessageStyle = useChatUiStore((s) => s.userMessageStyle);
  const compactTranscript = useChatUiStore((s) => s.desktopCompactTranscript);
  const timeIso = useLineTimeIso(msg.lineIndex);
  const body = <TurnMessageBody msg={msg} framed />;

  if (userMessageStyle === "bubble") {
    return (
      <article className="chat-user-turn is-bubble">
        <UserBubbleFrame timeIso={timeIso}>{body}</UserBubbleFrame>
      </article>
    );
  }

  return (
    <article className="chat-user-turn">
      <MessageTurnFrame role="user" name="你" timeIso={timeIso} compact={compactTranscript}>
        {body}
      </MessageTurnFrame>
    </article>
  );
}

export default function AgentTurnCard({
  turn,
  mcpPrefixes,
  renderBlock,
}: {
  turn: AgentTurn;
  mcpPrefixes: { id: string; prefix: string }[];
  renderBlock: (props: BlockRendererProps) => React.ReactNode;
}) {
  const historyParts = useStore((s) => s.chat_history_turn_parts);
  const compactTranscript = useChatUiStore((s) => s.desktopCompactTranscript);
  const agentParts = useMemo(
    () => resolveHistoryAgentParts(turn, historyParts),
    [turn, historyParts],
  );
  const { processParts } = useMemo(() => splitTurnParts(agentParts), [agentParts]);
  const stats = partsProcessStats(processParts);
  const durationMs = turnProcessDurationMs(turn.process);
  const failedTools = useMemo(
    () => countFailedToolGroups(turn.process),
    [turn.process],
  );
  const summary = formatTurnProcessSummary(stats, durationMs, failedTools);
  const turnMeta = turnMetaFromAgentTurn(turn);
  const steps = useMemo(
    () => partsToDisplaySteps(processParts, mcpPrefixes),
    [processParts, mcpPrefixes],
  );
  const hasProcess = steps.length > 0;

  const answerMsg = turn.answer?.message;
  const isLastAssistant = turn.answer?.isLastAssistant;
  const timeIso = useLineTimeIso(
    answerMsg?.lineIndex ?? turn.user?.message?.lineIndex ?? -1,
  );

  // While a turn is in flight, process/answer live in LiveZone — skip an empty
  // 「助手」frame so history + stream read as one continuous turn.
  if (!hasProcess && !answerMsg) return null;

  const answerIsError = answerMsg?.role === "error";

  return (
    <article className={`chat-agent-turn${answerIsError ? " chat-error-turn" : ""}`}>
      <MessageTurnFrame
        role={answerIsError ? "error" : "agent"}
        name={answerIsError ? "错误" : "助手"}
        timeIso={timeIso}
        compact={compactTranscript}
        footer={
          answerMsg && !answerIsError ? (
            <>
              {turnMeta && <AssistantTurnMeta text={turnMeta} />}
              <AgentMessageActions msg={answerMsg} isLastAssistant={isLastAssistant} />
            </>
          ) : undefined
        }
        footerPinned={!!isLastAssistant && !answerIsError}
      >
        {hasProcess && (
          <TurnProcessPanel
            steps={steps}
            summary={summary}
            stats={stats}
            mcpPrefixes={mcpPrefixes}
            renderBlock={renderBlock}
            defaultCollapsed
            variant="history"
          />
        )}

        {answerMsg &&
          (answerIsError ? (
            <ErrorMessageBody body={answerMsg.body} framed />
          ) : (
            <TurnMessageBody msg={answerMsg} framed />
          ))}
      </MessageTurnFrame>
    </article>
  );
}

export function AgentMessageActions({
  msg,
  isLastAssistant,
}: {
  msg: ChatMessage;
  isLastAssistant?: boolean;
}) {
  const [copied, setCopied] = useState(false);
  const chatBusy = useStore((s) => s.chat_busy);
  const assistantId = useStore((s) => s.chat_assistant_ids[String(msg.lineIndex)]);

  const onCopy = async () => {
    try {
      await navigator.clipboard.writeText(msg.body);
      setCopied(true);
      setTimeout(() => setCopied(false), 1200);
    } catch {
      /* clipboard unavailable */
    }
  };

  return (
    <>
      <button
        type="button"
        className="chat-agent-action"
        onClick={onCopy}
        aria-label="复制消息"
        title="复制"
      >
        {copied ? <Check size={15} /> : <Copy size={15} />}
      </button>
      {assistantId && !chatBusy && (
        <button
          type="button"
          className="chat-agent-action"
          onClick={() => void apiPost("/api/chat/regenerate", { message_id: assistantId })}
          aria-label={isLastAssistant ? "重新生成回复" : "从此消息重新生成"}
          title={isLastAssistant ? "重新生成" : "重新生成"}
        >
          <RefreshCw size={15} />
        </button>
      )}
    </>
  );
}
