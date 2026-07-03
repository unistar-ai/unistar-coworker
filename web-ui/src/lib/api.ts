import { authHeaders } from "./auth";

// Thin fetch wrapper that injects the auth Bearer header and surfaces errors
// as a typed result so callers can render a status toast without try/catch.

export interface ApiResult<T> {
  ok: boolean;
  status: number;
  data: T | null;
  error: string | null;
}

export async function apiFetch<T = unknown>(
  url: string,
  options: RequestInit = {},
): Promise<ApiResult<T>> {
  try {
    const merged: RequestInit = { ...options };
    merged.headers = authHeaders(
      (options.headers as Record<string, string>) || {},
    );
    const res = await fetch(url, merged);
    if (!res.ok) {
      const text = await res.text().catch(() => "");
      return {
        ok: false,
        status: res.status,
        data: null,
        error: `${res.status} ${res.statusText}${text ? `: ${text.slice(0, 200)}` : ""}`,
      };
    }
    // 204 No Content or empty body
    if (res.status === 204) {
      return { ok: true, status: 204, data: null, error: null };
    }
    const ct = res.headers.get("content-type") || "";
    if (ct.includes("application/json")) {
      const data = (await res.json()) as T;
      return { ok: true, status: res.status, data, error: null };
    }
    const text = await res.text();
    return { ok: true, status: res.status, data: text as unknown as T, error: null };
  } catch (e) {
    return {
      ok: false,
      status: 0,
      data: null,
      error: String((e as Error)?.message || e),
    };
  }
}

export async function apiPost(url: string, body?: unknown): Promise<ApiResult<unknown>> {
  return apiFetch(url, {
    method: "POST",
    headers: body != null ? { "Content-Type": "application/json" } : {},
    body: body != null ? JSON.stringify(body) : undefined,
  });
}

export async function apiDelete(url: string): Promise<ApiResult<unknown>> {
  return apiFetch(url, { method: "DELETE" });
}
