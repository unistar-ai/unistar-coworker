import type { ChatHistoryItem } from "./parser";

/** Cherry agent list uses 400px turn estimate; standalone blocks are shorter. */
export const TURN_ESTIMATE_SIZE_PX = 400;
export const BLOCK_ESTIMATE_SIZE_PX = 120;
export const VIRTUAL_OVERSCAN = 8;
export const VIRTUAL_THRESHOLD = 100;
export const STICK_BOTTOM_GAP_PX = 80;
export const PIN_RELEASE_TOLERANCE_PX = 24;

export function itemKey(item: ChatHistoryItem): string {
  return item.type === "block" ? item.block.key : item.turn.key;
}

export function estimateItemSize(item: ChatHistoryItem | undefined): number {
  if (!item) return TURN_ESTIMATE_SIZE_PX;
  return item.type === "turn" ? TURN_ESTIMATE_SIZE_PX : BLOCK_ESTIMATE_SIZE_PX;
}

export function isNewUserTurn(item: ChatHistoryItem, isNewKey: boolean): boolean {
  return item.type === "turn" && Boolean(item.turn.user) && isNewKey;
}

export function scrollItemIntoView(
  key: string,
  align: ScrollLogicalPosition = "start",
): void {
  const el = document.querySelector(`[data-block-key="${key}"]`);
  el?.scrollIntoView({ block: align, behavior: "instant" });
}

export function isPinnedItemNearTop(
  scrollEl: HTMLElement,
  itemKeyValue: string,
  tolerancePx = PIN_RELEASE_TOLERANCE_PX,
): boolean {
  const itemEl = scrollEl.querySelector(`[data-block-key="${itemKeyValue}"]`);
  if (!itemEl) return false;
  const itemRect = itemEl.getBoundingClientRect();
  const containerRect = scrollEl.getBoundingClientRect();
  return Math.abs(itemRect.top - containerRect.top) <= tolerancePx;
}
