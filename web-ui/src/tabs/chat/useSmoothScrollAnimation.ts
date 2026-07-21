import { type RefObject, useCallback, useEffect, useMemo, useRef } from "react";

export interface SmoothScrollOptions {
  frames?: number;
  easing?: (t: number) => number;
}

export interface SmoothFollowOptions {
  maxStep?: number;
  minStep?: number;
  damping?: number;
  settleThreshold?: number;
}

export interface SmoothScrollController {
  scrollTo(getTargetOffset: () => number, options?: SmoothScrollOptions): void;
  followTo(getTargetOffset: () => number, options?: SmoothFollowOptions): void;
  cancel(): void;
  isAnimating(): boolean;
}

const DEFAULT_FRAMES = 50;
const DEFAULT_EASING = (t: number): number => 1 - 2 ** (-10 * t);
const DEFAULT_FOLLOW_MAX_STEP = 64;
const DEFAULT_FOLLOW_MIN_STEP = 4;
const DEFAULT_FOLLOW_DAMPING = 0.32;
const DEFAULT_FOLLOW_SETTLE_THRESHOLD = 1;

function prefersReducedMotion(): boolean {
  try {
    return (
      typeof window !== "undefined" &&
      typeof window.matchMedia === "function" &&
      window.matchMedia("(prefers-reduced-motion: reduce)").matches
    );
  } catch {
    return false;
  }
}

type RafLike = (cb: FrameRequestCallback) => number;
type CafLike = (handle: number) => void;

interface UseSmoothScrollAnimationOptions {
  raf?: RafLike;
  caf?: CafLike;
}

/** RAF-driven scroll — suitable for streaming follow without native smooth scroll races. */
export function useSmoothScrollAnimation(
  scrollerRef: RefObject<HTMLElement | null>,
  { raf, caf }: UseSmoothScrollAnimationOptions = {},
): SmoothScrollController {
  const rafIdRef = useRef<number | null>(null);
  const animatingRef = useRef(false);

  const requestFrame = useMemo<RafLike>(
    () => raf ?? ((cb) => requestAnimationFrame(cb)),
    [raf],
  );
  const cancelFrame = useMemo<CafLike>(
    () => caf ?? ((id) => cancelAnimationFrame(id)),
    [caf],
  );

  const cancel = useCallback(() => {
    if (rafIdRef.current != null) {
      cancelFrame(rafIdRef.current);
      rafIdRef.current = null;
    }
    animatingRef.current = false;
  }, [cancelFrame]);

  const scrollTo = useCallback(
    (getTargetOffset: () => number, options: SmoothScrollOptions = {}) => {
      const el = scrollerRef.current;
      if (!el) return;

      if (prefersReducedMotion()) {
        if (rafIdRef.current != null) cancelFrame(rafIdRef.current);
        rafIdRef.current = null;
        animatingRef.current = false;
        el.scrollTop = getTargetOffset();
        return;
      }

      if (rafIdRef.current != null) cancelFrame(rafIdRef.current);

      const frames = Math.max(1, options.frames ?? DEFAULT_FRAMES);
      const easing = options.easing ?? DEFAULT_EASING;
      const startOffset = el.scrollTop;
      let frame = 0;

      animatingRef.current = true;

      const step = (): void => {
        const node = scrollerRef.current;
        if (!node) {
          animatingRef.current = false;
          rafIdRef.current = null;
          return;
        }

        frame += 1;
        const progress = Math.min(1, easing(frame / frames));
        const target = getTargetOffset();
        node.scrollTop = startOffset + (target - startOffset) * progress;

        if (frame >= frames) {
          node.scrollTop = getTargetOffset();
          animatingRef.current = false;
          rafIdRef.current = null;
          return;
        }

        rafIdRef.current = requestFrame(step);
      };

      rafIdRef.current = requestFrame(step);
    },
    [cancelFrame, requestFrame, scrollerRef],
  );

  const followTo = useCallback(
    (getTargetOffset: () => number, options: SmoothFollowOptions = {}) => {
      const el = scrollerRef.current;
      if (!el) return;

      if (prefersReducedMotion()) {
        el.scrollTop = getTargetOffset();
        return;
      }

      if (rafIdRef.current != null) return;

      const maxStep = Math.max(1, options.maxStep ?? DEFAULT_FOLLOW_MAX_STEP);
      const minStep = Math.min(maxStep, Math.max(1, options.minStep ?? DEFAULT_FOLLOW_MIN_STEP));
      const damping = Math.max(0.01, Math.min(1, options.damping ?? DEFAULT_FOLLOW_DAMPING));
      const settleThreshold = Math.max(0, options.settleThreshold ?? DEFAULT_FOLLOW_SETTLE_THRESHOLD);

      animatingRef.current = true;

      const step = (): void => {
        const node = scrollerRef.current;
        if (!node) {
          animatingRef.current = false;
          rafIdRef.current = null;
          return;
        }

        const target = getTargetOffset();
        const remaining = target - node.scrollTop;
        const distance = Math.abs(remaining);

        if (distance <= settleThreshold) {
          node.scrollTop = target;
          animatingRef.current = false;
          rafIdRef.current = null;
          return;
        }

        const magnitude = Math.min(
          distance,
          Math.min(maxStep, Math.max(minStep, distance * damping)),
        );
        node.scrollTop += Math.sign(remaining) * magnitude;
        rafIdRef.current = requestFrame(step);
      };

      rafIdRef.current = requestFrame(step);
    },
    [requestFrame, scrollerRef],
  );

  const isAnimating = useCallback(() => animatingRef.current, []);

  useEffect(() => () => cancel(), [cancel]);

  return useMemo(
    () => ({ scrollTo, followTo, cancel, isAnimating }),
    [cancel, followTo, isAnimating, scrollTo],
  );
}
