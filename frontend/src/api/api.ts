// Central API access layer. When the server requires a web bearer token
// (HATCHDOOR_WEB_BEARER_TOKEN), it is stored locally and attached to every
// request — as an Authorization header for fetch, or as an `access_token`
// query parameter for contexts that cannot set headers (<img>, downloads, SSE).

const TOKEN_KEY = "hatchdoor_web_token";
export const DEFAULT_FETCH_TIMEOUT_MS = 15_000;

// The token for this page only, held when the browser refuses to store it.
// WebKit with site data blocked throws from every localStorage access, and
// without this an Unlock would forget the token at once and every request
// would stay on 401 (#339). Null whenever storage accepted the token, so a
// sign-out in another tab still applies here.
let unstoredToken: string | null = null;

export function getToken(): string | null {
  try {
    return localStorage.getItem(TOKEN_KEY) ?? unstoredToken;
  } catch {
    return unstoredToken;
  }
}

/** Remember the web token. Returns false when the browser refused to store
 * it: the token then lasts only as long as this page, so a caller must not
 * reload to apply it. */
export function setToken(token: string): boolean {
  try {
    localStorage.setItem(TOKEN_KEY, token);
    unstoredToken = null;
    return true;
  } catch {
    unstoredToken = token;
    return false;
  }
}

export function clearToken(): void {
  unstoredToken = null;
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

/** `RequestInit` plus an optional per-call timeout override. Slow-by-nature
 * requests (attachment uploads, note mutations on large vaults) need more than
 * the default read timeout. */
export type ApiFetchInit = RequestInit & { timeoutMs?: number };

/**
 * Fetch wrapper that attaches the bearer token when one is stored and notifies
 * the unauthorized handler on a 401. When no token is stored the call is
 * forwarded unchanged so unauthenticated deployments behave exactly as before.
 */
export async function apiFetch(
  input: RequestInfo | URL,
  init?: ApiFetchInit,
): Promise<Response> {
  const { timeoutMs = DEFAULT_FETCH_TIMEOUT_MS, ...requestInit } = init ?? {};
  const token = getToken();
  const timeoutController = new AbortController();
  const timeoutId = window.setTimeout(() => {
    timeoutController.abort(
      new DOMException("Request timed out", "AbortError"),
    );
  }, timeoutMs);
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

  let finalInit: RequestInit = requestInit;
  if (token) {
    const headers = new Headers(requestInit.headers);
    headers.set("Authorization", `Bearer ${token}`);
    finalInit = {
      ...requestInit,
      headers,
    };
  }

  try {
    const res = await fetch(input, {
      ...finalInit,
      signal: timeoutController.signal,
    });
    // A 401 for a request sent with a token that has since changed says
    // nothing about the new one. An Unlock applied in place (#339) would
    // otherwise be locked again by the answers still in flight from before it.
    if (res.status === 401 && getToken() === token) {
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
