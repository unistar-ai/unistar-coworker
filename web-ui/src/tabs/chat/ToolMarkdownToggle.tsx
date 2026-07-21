import type { MouseEvent } from "react";
import { useChatUiStore } from "../../store/chatUiStore";

/** Compact MD / 原文 toggle for tool result pane headers. */
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
          ? "工具结果：Markdown 渲染（点击切换为原文）"
          : "工具结果：原文（点击切换为 Markdown 渲染）"
      }
      aria-pressed={toolMarkdown}
    >
      {toolMarkdown ? "MD" : "原文"}
    </button>
  );
}
