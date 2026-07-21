import { useEffect, useState } from "react";

/** Matches Cherry `MessagePartsRenderer` trailing result hold. */
export const TRAILING_RESULT_RELEASE_DELAY_MS = 2000;

/**
 * Whether the streaming answer body should render below the process fold.
 * During active process, answer is held; after process ends, wait 2s if tools/reasoning ran.
 */
export function shouldShowStreamingAnswer(
  processActive: boolean,
  streaming: string | null | undefined,
  released: boolean,
  hadProcess: boolean,
): boolean {
  if (!streaming) return false;
  if (processActive) return false;
  if (!hadProcess) return true;
  return released;
}

export function useTrailingAnswerRelease(
  processActive: boolean,
  streaming: string | null | undefined,
  turnActive: boolean,
): boolean {
  const hasStreaming = Boolean(streaming);
  const [hadProcess, setHadProcess] = useState(false);
  const [released, setReleased] = useState(true);

  useEffect(() => {
    if (processActive) setHadProcess(true);
  }, [processActive]);

  useEffect(() => {
    if (!turnActive) {
      setHadProcess(false);
      setReleased(true);
      return;
    }
    if (processActive || !hasStreaming) {
      setReleased(false);
      return;
    }
    if (!hadProcess) {
      setReleased(true);
      return;
    }
    const timer = window.setTimeout(() => setReleased(true), TRAILING_RESULT_RELEASE_DELAY_MS);
    return () => window.clearTimeout(timer);
  }, [turnActive, processActive, hasStreaming, hadProcess]);

  return shouldShowStreamingAnswer(processActive, streaming, released, hadProcess);
}
