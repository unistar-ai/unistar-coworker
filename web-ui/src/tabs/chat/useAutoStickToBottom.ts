import { type RefObject, useCallback, useMemo, useRef } from "react";
import { getDistanceToBottom, getRealBottom } from "./scrollGeometry";
import type { SmoothScrollController } from "./useSmoothScrollAnimation";

const BOTTOM_FOLLOW_MIN_STEP_PX = 4;
const BOTTOM_FOLLOW_DAMPING = 0.32;
const BOTTOM_FOLLOW_SNAP_DISTANCE_VIEWPORTS = 3;

function getBottomFollowMaxStep(element: HTMLElement): number {
  return Math.max(48, Math.min(96, element.clientHeight * 0.12));
}

export interface AutoStickInputs {
  scrollerRef: RefObject<HTMLElement | null>;
  getBottomInset?: () => number;
  smoothScroll: SmoothScrollController;
  isAtBottom(): boolean;
  isLocked(): boolean;
  markStuck(): void;
}

export interface AutoStickToBottom {
  onContentSizeChange(): void;
}

export function useAutoStickToBottom({
  scrollerRef,
  getBottomInset,
  smoothScroll,
  isAtBottom,
  isLocked,
  markStuck,
}: AutoStickInputs): AutoStickToBottom {
  const lastScrollSizeRef = useRef(0);

  const targetBottom = useCallback(() => {
    const el = scrollerRef.current;
    if (!el) return 0;
    return getRealBottom(el, getBottomInset?.() ?? 0);
  }, [getBottomInset, scrollerRef]);

  const onContentSizeChange = useCallback(() => {
    const el = scrollerRef.current;
    if (!el) return;
    const prev = lastScrollSizeRef.current;
    const curr = el.scrollHeight;
    if (curr === prev) return;
    lastScrollSizeRef.current = curr;
    if (isLocked()) return;
    if (!isAtBottom()) return;
    if (curr <= prev) return;
    if (smoothScroll.isAnimating()) return;

    const inset = getBottomInset?.() ?? 0;
    if (getDistanceToBottom(el, inset) > el.clientHeight * BOTTOM_FOLLOW_SNAP_DISTANCE_VIEWPORTS) {
      el.scrollTop = targetBottom();
      markStuck();
      return;
    }

    smoothScroll.followTo(targetBottom, {
      maxStep: getBottomFollowMaxStep(el),
      minStep: BOTTOM_FOLLOW_MIN_STEP_PX,
      damping: BOTTOM_FOLLOW_DAMPING,
    });
    markStuck();
  }, [getBottomInset, isAtBottom, isLocked, markStuck, scrollerRef, smoothScroll, targetBottom]);

  return useMemo(() => ({ onContentSizeChange }), [onContentSizeChange]);
}
