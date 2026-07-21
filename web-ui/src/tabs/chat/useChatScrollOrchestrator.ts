import { useCallback, useEffect, useRef } from "react";
import { getRealBottom } from "./scrollGeometry";
import { isPinnedItemReleased, useScrollPinAnchor } from "./useScrollPinAnchor";
import { useAutoStickToBottom } from "./useAutoStickToBottom";
import { useSmoothScrollAnimation } from "./useSmoothScrollAnimation";
import { STICK_BOTTOM_GAP_PX } from "./chatScroll";

export interface ChatScrollOrchestrator {
  spacerPx: number;
  pinTo: (key: string, scrollToVirtual?: (key: string) => void) => void;
  releasePin: () => void;
  pinnedKeyRef: React.MutableRefObject<string | null>;
  userScrollRef: React.MutableRefObject<boolean>;
  notifyContentChange: () => void;
  scrollToBottom: (mode?: "instant" | "smooth" | "follow") => void;
  attachScrollElement: (el: HTMLElement) => () => void;
}

/**
 * Cherry-style scroll orchestration: pin anchor yields to nothing; stick-bottom
 * uses smooth follow when content grows during streaming.
 */
export function useChatScrollOrchestrator({
  scrollRef,
  stickBottom,
  onStickBottomChange,
}: {
  scrollRef: React.RefObject<HTMLElement | null>;
  stickBottom: boolean;
  onStickBottomChange?: (stick: boolean) => void;
}): ChatScrollOrchestrator {
  const stickBottomRef = useRef(stickBottom);
  stickBottomRef.current = stickBottom;

  const smoothScroll = useSmoothScrollAnimation(scrollRef);
  const {
    spacerPx,
    pinTo,
    release: releasePin,
    isPinned,
    onPinResize,
    pinnedKeyRef,
    userScrollRef,
  } = useScrollPinAnchor(scrollRef);

  const autoStick = useAutoStickToBottom({
    scrollerRef: scrollRef,
    smoothScroll,
    isAtBottom: () => stickBottomRef.current,
    isLocked: isPinned,
    markStuck: () => onStickBottomChange?.(true),
  });

  const notifyContentChange = useCallback(() => {
    onPinResize();
    autoStick.onContentSizeChange();
  }, [autoStick, onPinResize]);

  const scrollToBottom = useCallback(
    (mode: "instant" | "smooth" | "follow" = "follow") => {
      if (isPinned()) return;
      const el = scrollRef.current;
      if (!el) return;
      const target = () => getRealBottom(el);
      if (mode === "instant") {
        smoothScroll.cancel();
        el.scrollTop = target();
      } else if (mode === "smooth") {
        smoothScroll.scrollTo(target);
      } else {
        smoothScroll.followTo(target);
      }
      onStickBottomChange?.(true);
    },
    [isPinned, onStickBottomChange, scrollRef, smoothScroll],
  );

  const attachScrollElement = useCallback(
    (el: HTMLElement) => {
      const markUserScroll = () => {
        userScrollRef.current = true;
      };
      const onScroll = () => {
        const gap = el.scrollHeight - el.scrollTop - el.clientHeight;
        onStickBottomChange?.(gap < STICK_BOTTOM_GAP_PX);
        const pinnedKey = pinnedKeyRef.current;
        if (pinnedKey && userScrollRef.current) {
          if (isPinnedItemReleased(el, pinnedKey)) {
            releasePin();
          }
        }
      };
      const onWheel = (e: WheelEvent) => {
        if (e.deltaY < 0) smoothScroll.cancel();
        markUserScroll();
      };

      el.addEventListener("scroll", onScroll, { passive: true });
      el.addEventListener("wheel", onWheel, { passive: true });
      el.addEventListener("touchstart", markUserScroll, { passive: true });

      const ro = new ResizeObserver(() => notifyContentChange());
      ro.observe(el);
      const history = el.querySelector(".msg-history");
      if (history) ro.observe(history);
      const blockList = el.querySelector(".chat-block-list");
      if (blockList) ro.observe(blockList);
      const liveZone = el.querySelector(".live-zone");
      if (liveZone) ro.observe(liveZone);

      return () => {
        el.removeEventListener("scroll", onScroll);
        el.removeEventListener("wheel", onWheel);
        el.removeEventListener("touchstart", markUserScroll);
        ro.disconnect();
        smoothScroll.cancel();
      };
    },
    [
      notifyContentChange,
      onStickBottomChange,
      pinnedKeyRef,
      releasePin,
      smoothScroll,
      userScrollRef,
    ],
  );

  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    return attachScrollElement(el);
  }, [attachScrollElement, scrollRef]);

  return {
    spacerPx,
    pinTo,
    releasePin,
    pinnedKeyRef,
    userScrollRef,
    notifyContentChange,
    scrollToBottom,
    attachScrollElement,
  };
}
