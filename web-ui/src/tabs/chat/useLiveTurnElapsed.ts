import { useEffect, useState } from "react";

/** Wall-clock elapsed for the in-flight chat turn (Live process summary). */
export function useLiveTurnElapsed(chatBusy: boolean): number | null {
  const [elapsedMs, setElapsedMs] = useState(0);

  useEffect(() => {
    if (!chatBusy) {
      setElapsedMs(0);
      return;
    }
    const started = Date.now();
    setElapsedMs(0);
    const id = window.setInterval(() => setElapsedMs(Date.now() - started), 250);
    return () => window.clearInterval(id);
  }, [chatBusy]);

  return chatBusy ? elapsedMs : null;
}
