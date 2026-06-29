// Auth token handling: read ?token= from the URL on first load, stash it in
// sessionStorage, strip from URL, and expose helpers to attach the Bearer
// header to fetch requests and the token query to WebSocket URLs.

const TOKEN_KEY = "unistar-web-token";

export function readTokenFromUrl(): string | null {
  try {
    const params = new URLSearchParams(location.search);
    const t = params.get("token");
    if (t) {
      sessionStorage.setItem(TOKEN_KEY, t);
      const cleanUrl = location.pathname + location.hash;
      history.replaceState(null, "", cleanUrl);
      return t;
    }
  } catch {
    /* sessionStorage / history unavailable */
  }
  return null;
}

export function getAuthToken(): string | null {
  try {
    return sessionStorage.getItem(TOKEN_KEY);
  } catch {
    return null;
  }
}

export function authHeaders(extra: Record<string, string> = {}): Record<string, string> {
  const token = getAuthToken();
  const headers = { ...extra };
  if (token) {
    headers["Authorization"] = `Bearer ${token}`;
  }
  return headers;
}

export function wsTokenQuery(): string {
  const token = getAuthToken();
  return token ? `?token=${encodeURIComponent(token)}` : "";
}
