import { useCallback, useRef, useState } from "react";
import {
  PIN_RELEASE_TOLERANCE_PX,
  scrollItemIntoView,
} from "./chatScroll";

const REASSERT_TOLERANCE_PX = 2;

export interface ScrollPinAnchor {
  spacerPx: number;
  pinTo: (key: string, scrollToVirtual?: (key: string) => void) => void;
  release: () => void;
  isPinned: () => boolean;
  onPinResize: () => void;
  pinnedKeyRef: React.MutableRefObject<string | null>;
  userScrollRef: React.MutableRefObject<boolean>;
}

/**
 * Cherry-style scroll pin: append a bottom spacer so the pinned item can sit at
 * the viewport top while content below grows (streaming). Reasserts on resize.
 */
export function useScrollPinAnchor(
  scrollRef: React.RefObject<HTMLElement | null>,
): ScrollPinAnchor {
  const [spacerPx, setSpacerPx] = useState(0);
  const pinnedKeyRef = useRef<string | null>(null);
  const userScrollRef = useRef(false);
  const anchorScrollTopRef = useRef(0);
  const lastNaturalHeightRef = useRef(0);

  const release = useCallback(() => {
    pinnedKeyRef.current = null;
    setSpacerPx(0);
    lastNaturalHeightRef.current = 0;
  }, []);

  const pinTo = useCallback(
    (key: string, scrollToVirtual?: (key: string) => void) => {
      const el = scrollRef.current;
      if (!el) return;
      pinnedKeyRef.current = key;
      userScrollRef.current = false;
      const viewport = el.clientHeight;
      setSpacerPx(viewport);
      requestAnimationFrame(() => {
        if (scrollToVirtual) scrollToVirtual(key);
        else scrollItemIntoView(key, "start");
        anchorScrollTopRef.current = el.scrollTop;
        lastNaturalHeightRef.current = el.scrollHeight - viewport;
      });
    },
    [scrollRef],
  );

  const reassertPin = useCallback(() => {
    const el = scrollRef.current;
    const key = pinnedKeyRef.current;
    if (!el || !key || userScrollRef.current) return;

    const itemEl = el.querySelector(`[data-block-key="${key}"]`);
    if (!itemEl) return;

    const containerRect = el.getBoundingClientRect();
    const itemRect = itemEl.getBoundingClientRect();
    const drift = itemRect.top - containerRect.top;
    if (Math.abs(drift) > REASSERT_TOLERANCE_PX) {
      el.scrollTop += drift;
      anchorScrollTopRef.current = el.scrollTop;
    }

    const naturalHeight = el.scrollHeight - spacerPx;
    const contentGrew = naturalHeight > lastNaturalHeightRef.current;
    lastNaturalHeightRef.current = naturalHeight;

    const needed = Math.max(0, anchorScrollTopRef.current + el.clientHeight - naturalHeight);
    if (needed === 0) {
      release();
      return;
    }
    if (needed > spacerPx) {
      setSpacerPx(needed);
    } else if (needed < spacerPx && !contentGrew) {
      setSpacerPx(needed);
    }
  }, [release, scrollRef, spacerPx]);

  const isPinned = useCallback(() => pinnedKeyRef.current !== null, []);

  return {
    spacerPx,
    pinTo,
    release,
    isPinned,
    onPinResize: reassertPin,
    pinnedKeyRef,
    userScrollRef,
  };
}

export function isPinnedItemReleased(
  scrollEl: HTMLElement,
  itemKeyValue: string,
  tolerancePx = PIN_RELEASE_TOLERANCE_PX,
): boolean {
  const itemEl = scrollEl.querySelector(`[data-block-key="${itemKeyValue}"]`);
  if (!itemEl) return true;
  const itemRect = itemEl.getBoundingClientRect();
  const containerRect = scrollEl.getBoundingClientRect();
  return Math.abs(itemRect.top - containerRect.top) > tolerancePx;
}
