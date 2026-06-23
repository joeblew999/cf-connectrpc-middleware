import { ConnectError } from "@connectrpc/connect";
import type { Transport } from "@connectrpc/connect";
import { createConnectTransport } from "@connectrpc/connect-web";

/** Pull a human-readable message out of a ConnectError, else `fallback`. */
export function errorMessage(e: unknown, fallback: string): string {
  return e instanceof ConnectError ? e.message : fallback;
}

let currentToken: string | null = null;

/**
 * Set the bearer token stamped on every subsequent request by a transport
 * from {@link makeTransport}. Your auth layer calls this on login/logout.
 */
export function setAuthToken(token: string | null): void {
  currentToken = token;
}

/**
 * Build a Connect transport that stamps `Authorization: Bearer <token>` (the
 * token set via {@link setAuthToken}) — matching a server-side bearer/token
 * auth layer. `baseUrl` is your ConnectRPC service URL.
 */
export function makeTransport(baseUrl: string): Transport {
  return createConnectTransport({
    baseUrl,
    fetch: ((input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
      const headers = new Headers(init?.headers);
      if (currentToken && !headers.has("Authorization")) {
        headers.set("Authorization", `Bearer ${currentToken}`);
      }
      return globalThis.fetch(input, { ...init, headers });
    }) as typeof globalThis.fetch,
  });
}
