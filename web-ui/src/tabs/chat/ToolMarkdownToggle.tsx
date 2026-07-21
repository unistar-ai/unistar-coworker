import type { MouseEvent } from "react";
import { useChatUiStore } from "../../store/chatUiStore";

/** Compact MD / 原文 toggle — only mount when the result can use Markdown. */
export default function ToolMarkdownToggle({ className }: { className?: string }) {
  const toolMarkdown = useChatUiStore((s) => s.toolMarkdown);
  const toggleToolMarkdown = useChatUiStore((s) => s.toggleToolMarkdown);

  const onClick = (e: MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    toggleToolMarkdown();
  };

  return (
    <button
      type="button"
      className={`tool-md-toggle${toolMarkdown ? " is-active" : ""}${className ? ` ${className}` : ""}`}
      onClick={onClick}
      title={
        toolMarkdown
          ? "当前以 Markdown 渲染此结果（点击改为原文）"
          : "当前显示原文（点击改为 Markdown 渲染）"
      }
      aria-pressed={toolMarkdown}
    >
      {toolMarkdown ? "Markdown" : "原文"}
    </button>
  );
}
