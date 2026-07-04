// Central API access layer. When the server requires a web bearer token
// (HATCHDOOR_WEB_BEARER_TOKEN), it is stored locally and attached to every
// request — as an Authorization header for fetch, or as an `access_token`
// query parameter for contexts that cannot set headers (<img>, downloads, SSE).

const TOKEN_KEY = "hatchdoor_web_token";
export const DEFAULT_FETCH_TIMEOUT_MS = 15_000;

export function getToken(): string | null {
  try {
    return localStorage.getItem(TOKEN_KEY);
  } catch {
    return null;
  }
}

export function setToken(token: string): void {
  try {
    localStorage.setItem(TOKEN_KEY, token);
  } catch {
    // Ignore storage failures (private mode, disabled storage).
  }
}

export function clearToken(): void {
  try {
    localStorage.removeItem(TOKEN_KEY);
  } catch {
    // Ignore.
  }
}

type UnauthorizedHandler = () => void;
let unauthorizedHandler: UnauthorizedHandler | null = null;

/** Register a callback fired whenever a request comes back 401. */
export function onUnauthorized(handler: UnauthorizedHandler | null): void {
  unauthorizedHandler = handler;
}

/**
 * Fetch wrapper that attaches the bearer token when one is stored and notifies
 * the unauthorized handler on a 401. When no token is stored the call is
 * forwarded unchanged so unauthenticated deployments behave exactly as before.
 */
export async function apiFetch(
  input: RequestInfo | URL,
  init?: RequestInit,
): Promise<Response> {
  const token = getToken();
  const timeoutController = new AbortController();
  const timeoutId = window.setTimeout(() => {
    timeoutController.abort(
      new DOMException("Request timed out", "AbortError"),
    );
  }, DEFAULT_FETCH_TIMEOUT_MS);
  const callerSignal = init?.signal;
  const abortFromCaller = () => {
    timeoutController.abort(
      callerSignal?.reason ?? new DOMException("Aborted", "AbortError"),
    );
  };
  if (callerSignal?.aborted) {
    abortFromCaller();
  } else {
    callerSignal?.addEventListener("abort", abortFromCaller, { once: true });
  }

  let finalInit = init;
  if (token) {
    finalInit = {
      ...init,
      headers: {
        ...((init?.headers as Record<string, string>) ?? {}),
        Authorization: `Bearer ${token}`,
      },
    };
  }

  try {
    const res = await fetch(input, {
      ...finalInit,
      signal: timeoutController.signal,
    });
    if (res.status === 401) {
      unauthorizedHandler?.();
    }
    return res;
  } finally {
    window.clearTimeout(timeoutId);
    callerSignal?.removeEventListener("abort", abortFromCaller);
  }
}

export function notifyUnauthorized(): void {
  unauthorizedHandler?.();
}

/**
 * Append the stored token as an `access_token` query parameter, for URLs used
 * where headers cannot be set (image src, download links, EventSource). Returns
 * the URL unchanged when no token is stored.
 */
export function withAccessToken(url: string): string {
  const token = getToken();
  if (!token) {
    return url;
  }
  const separator = url.includes("?") ? "&" : "?";
  return `${url}${separator}access_token=${encodeURIComponent(token)}`;
}
