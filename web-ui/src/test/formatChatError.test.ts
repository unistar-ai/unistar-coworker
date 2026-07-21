import { describe, expect, it } from "vitest";
import { formatChatError } from "../tabs/chat/formatChatError";

describe("formatChatError", () => {
  it("parses llm HTTP JSON body", () => {
    const raw =
      'llm HTTP 400 Bad Request: {"error":{"message":"An assistant message with \'tool_calls\' must be followed by tool messages responding to each \'tool_call_id\'. (insufficient tool messages following tool_calls message)","type":"invalid_request_error","param":null,"code":"invalid_request_error"}}';
    const f = formatChatError(raw);
    expect(f.title).toBe("模型请求失败");
    expect(f.status).toBe("400 Bad Request");
    expect(f.code).toBe("invalid_request_error");
    expect(f.message).toContain("tool_calls");
    expect(f.raw).toBe(raw);
  });

  it("falls back for plain text", () => {
    const f = formatChatError("connection refused");
    expect(f.title).toBe("出错了");
    expect(f.message).toBe("connection refused");
  });
});
