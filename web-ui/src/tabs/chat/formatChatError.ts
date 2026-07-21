/** Parse chat `error>` bodies into a calm, readable display shape. */

export interface FormattedChatError {
  /** Short headline, e.g. "模型请求失败". */
  title: string;
  /** Human-readable primary message. */
  message: string;
  /** Optional HTTP status when present. */
  status?: string;
  /** Optional provider error type/code. */
  code?: string;
  /** Raw body for expand-to-detail. */
  raw: string;
}

function tryParseJsonObject(text: string): Record<string, unknown> | null {
  const trimmed = text.trim();
  if (!trimmed.startsWith("{")) return null;
  try {
    const v = JSON.parse(trimmed) as unknown;
    if (v && typeof v === "object" && !Array.isArray(v)) {
      return v as Record<string, unknown>;
    }
  } catch {
    /* not JSON */
  }
  return null;
}

function extractNestedMessage(obj: Record<string, unknown>): {
  message?: string;
  type?: string;
  code?: string;
} {
  const err = obj.error;
  if (err && typeof err === "object" && !Array.isArray(err)) {
    const e = err as Record<string, unknown>;
    return {
      message: typeof e.message === "string" ? e.message : undefined,
      type: typeof e.type === "string" ? e.type : undefined,
      code: typeof e.code === "string" ? e.code : undefined,
    };
  }
  return {
    message: typeof obj.message === "string" ? obj.message : undefined,
    type: typeof obj.type === "string" ? obj.type : undefined,
    code: typeof obj.code === "string" ? obj.code : undefined,
  };
}

/** Format an `error>` chat line for the Web UI. */
export function formatChatError(raw: string): FormattedChatError {
  const body = (raw || "").trim();
  if (!body) {
    return { title: "出错了", message: "未知错误", raw: "" };
  }

  const httpMatch = body.match(/^llm HTTP\s+(\d{3})(?:\s+([^:]+))?:\s*([\s\S]*)$/i);
  if (httpMatch) {
    const status = httpMatch[1];
    const statusText = (httpMatch[2] || "").trim();
    const rest = httpMatch[3].trim();
    const json = tryParseJsonObject(rest);
    const nested = json ? extractNestedMessage(json) : {};
    const message =
      nested.message ||
      (json ? rest : rest) ||
      statusText ||
      "请求失败";
    return {
      title: "模型请求失败",
      message,
      status: statusText ? `${status} ${statusText}` : status,
      code: nested.code || nested.type,
      raw: body,
    };
  }

  const json = tryParseJsonObject(body);
  if (json) {
    const nested = extractNestedMessage(json);
    return {
      title: "出错了",
      message: nested.message || body,
      code: nested.code || nested.type,
      raw: body,
    };
  }

  // Truncate extremely long plain errors for the summary line.
  const message = body.length > 280 ? `${body.slice(0, 279)}…` : body;
  return { title: "出错了", message, raw: body };
}
