import { useEffect, useState } from "react";

/** Matches Cherry `MessagePartsRenderer` trailing result hold (history turns only). */
export const TRAILING_RESULT_RELEASE_DELAY_MS = 2000;

/**
 * Whether the streaming answer body should render in the live zone.
 * Show tokens as soon as `chat_streaming` updates — do not wait for tools/reasoning to finish.
 */
export function shouldShowStreamingAnswer(
  _processActive: boolean,
  streaming: string | null | undefined,
  _released: boolean,
  _hadProcess: boolean,
): boolean {
  return Boolean(streaming?.length);
}

/** @deprecated Release delay only applied when streaming without visible body; kept for API stability. */
export function useTrailingAnswerRelease(
  processActive: boolean,
  streaming: string | null | undefined,
  turnActive: boolean,
): boolean {
  const hasStreaming = Boolean(streaming?.length);
  const [hadProcess, setHadProcess] = useState(false);

  useEffect(() => {
    if (processActive) setHadProcess(true);
  }, [processActive]);

  useEffect(() => {
    if (!turnActive) setHadProcess(false);
  }, [turnActive]);

  if (!turnActive || !hasStreaming) return false;
  return shouldShowStreamingAnswer(processActive, streaming, true, hadProcess);
}
