// Streaming text splitter — divides a partial markdown stream into a
// "stable" prefix (safe to render as Markdown without layout jumps from
// unclosed code fences / tables / lists) and an "unstable" tail (rendered as
// plain text + a blinking cursor). The goal is to keep partial formatting
// visible (the user's stated requirement) while avoiding the jitter where an
// unclosed ``` swallows the rest of the stream until the closing fence
// arrives.
//
// Heuristic:
//  - Split on the last blank line (paragraph boundary). Everything up to and
//    including that boundary is stable. The tail after it is unstable.
//  - Additionally, if the stable part ends inside an unclosed fenced code
//    block (odd number of ``` / ~~~ fences), move the split back to before
//    the opening fence so the unclosed fence lives in the unstable tail.
//  - Keep at least the last ~2 lines in the tail so the most recently typed
//    content is always shown as plain text (no re-parse jitter on each token).

export interface StreamSplit {
  stable: string;
  unstable: string;
}

const FENCE_RE = /(^|\n)(`{3,}|~{3,})/g;

/** Count how many fenced-code opening markers are in `text`. A code block is
 * "open" when the count is odd (assuming each fence opens then closes a
 * block, which holds for well-formed markdown). */
function unclosedFence(text: string): boolean {
  let count = 0;
  FENCE_RE.lastIndex = 0;
  let m: RegExpExecArray | null;
  while ((m = FENCE_RE.exec(text)) !== null) {
    count += 1;
    // Avoid zero-width match infinite loop.
    if (m[0].length === 0) FENCE_RE.lastIndex++;
  }
  return count % 2 === 1;
}

export function splitStreaming(text: string): StreamSplit {
  if (!text) return { stable: "", unstable: "" };

  // 1. Split at the last paragraph boundary (a blank line).
  const blankIdx = text.lastIndexOf("\n\n");
  let splitAt = blankIdx >= 0 ? blankIdx + 2 : -1; // +2 to keep the boundary in stable

  // 2. If no blank line, fall back to keeping the last ~2 lines as unstable
  //    so the freshly-typed tail doesn't re-parse on every token.
  if (splitAt < 0) {
    const lines = text.split("\n");
    if (lines.length <= 2) {
      // Very short: all unstable (plain text), nothing stable yet.
      return { stable: "", unstable: text };
    }
    splitAt = text.length - lines.slice(-2).join("\n").length;
    // Realign to a line start.
    const nl = text.lastIndexOf("\n", splitAt - 1);
    splitAt = nl >= 0 ? nl + 1 : 0;
  }

  let stable = text.slice(0, splitAt);
  let unstable = text.slice(splitAt);

  // 3. If the stable part ends inside an unclosed code fence, pull the
  //    opening fence into the unstable tail so it doesn't swallow text.
  if (unclosedFence(stable)) {
    // Trim trailing whitespace so the fence opener (which is on its own line)
    // is reliably the last fence in stable, then find its start index.
    const trimmed = stable.replace(/\s+$/, "");
    const fenceMatch = /(^|\n)(`{3,}|~{3,})[^\n]*$/.exec(trimmed);
    if (fenceMatch) {
      const fenceStart = fenceMatch.index + (fenceMatch[1] ? fenceMatch[1].length : 0);
      unstable = stable.slice(fenceStart) + unstable;
      stable = stable.slice(0, fenceStart).replace(/\n+$/, "");
    }
  }

  return { stable, unstable };
}
