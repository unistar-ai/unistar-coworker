import type { ReactNode } from "react";
import {
  Brain,
  CalendarClock,
  CircleHelp,
  Code,
  Eye,
  FileText,
  Folder,
  Globe,
  MessageSquare,
  Puzzle,
  Search,
  Server,
  Terminal,
  Users,
  Wrench,
} from "lucide-react";

const ICON_SIZE = 15;
const STROKE = 2;

/** Lucide icon for a native or MCP tool name (yahu-style tool rows). */
export function ToolLucideIcon({
  toolName,
  size = ICON_SIZE,
}: {
  toolName: string;
  size?: number;
}): ReactNode {
  const name = (toolName || "").toLowerCase().replace(/^functions\./, "");
  const props = { size, strokeWidth: STROKE, "aria-hidden": true as const };

  if (name.startsWith("mcp_") || name.includes(".")) {
    return <Server {...props} />;
  }
  if (name === "bash_run" || name === "python_run") return <Terminal {...props} />;
  if (name === "read_file" || name === "write_file" || name === "edit_file") {
    return <FileText {...props} />;
  }
  if (name === "grep" || name === "glob" || name === "tool_search") {
    return <Search {...props} />;
  }
  if (name === "web_fetch" || name === "web_browser") return <Globe {...props} />;
  if (name.startsWith("skill")) return <Puzzle {...props} />;
  if (name === "ask_user") return <CircleHelp {...props} />;
  if (name === "memory") return <Brain {...props} />;
  if (name === "delegate_task") return <Users {...props} />;
  if (name.startsWith("pr_")) return <GitPrIcon {...props} />;
  if (name === "execute_code") return <Code {...props} />;
  if (name === "vision_analyze") return <Eye {...props} />;
  if (name === "cronjob") return <CalendarClock {...props} />;
  if (name === "discord") return <MessageSquare {...props} />;
  return <Wrench {...props} />;
}

function GitPrIcon(props: { size?: number; strokeWidth?: number; "aria-hidden"?: boolean }) {
  return <Folder {...props} />;
}
