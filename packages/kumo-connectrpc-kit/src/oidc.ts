/**
 * Rauthy OIDC client — the AuthN flows that mint the token `AuthProvider` holds.
 *
 * Two ways in, ONE token out (both produce the identical Rauthy JWT, which the
 * server-side `connectrpc-oidc` middleware verifies the same way — it never
 * sees the flow):
 *
 *   - `loginWithRedirect` / `handleRedirectCallback` — authorization_code + PKCE.
 *     The user authenticates on Rauthy's hosted page, so this is the ONLY flow
 *     that can do **passkeys, MFA, and social login** (they need the interactive
 *     page). Public client, no secret. This is the one you want for those.
 *   - `loginWithPassword` — ROPC password grant. Fully in-app/branded, no
 *     redirect, but **email+password only** (no passkeys/MFA). First-party.
 *
 * Endpoints are derived from `issuer` using Rauthy's layout
 * (`<issuer>/oidc/authorize`, `<issuer>/oidc/token`) unless overridden.
 */

export interface OidcConfig {
  /** The Rauthy issuer WITH path, e.g. `http://localhost:8080/auth/v1/`. */
  issuer: string;
  /** The registered OIDC client id, e.g. `worker-client`. */
  clientId: string;
  /** Where Rauthy redirects back after login, e.g. `http://localhost:5173/callback`. */
  redirectUri: string;
  /** Requested scopes. Default: `openid profile groups email`. */
  scopes?: string[];
  /** Override the authorize endpoint (else `<issuer>/oidc/authorize`). */
  authorizeUrl?: string;
  /** Override the token endpoint (else `<issuer>/oidc/token`). */
  tokenUrl?: string;
}

export interface TokenResponse {
  access_token: string;
  token_type: string;
  id_token?: string;
  refresh_token?: string;
  expires_in?: number;
  scope?: string;
}

const DEFAULT_SCOPES = ["openid", "profile", "groups", "email"];
const PKCE_KEY = "kit.oidc.pkce";

function endpoint(issuer: string, path: string): string {
  return issuer.replace(/\/+$/, "") + "/" + path;
}

function authorizeEndpoint(c: OidcConfig): string {
  return c.authorizeUrl ?? endpoint(c.issuer, "oidc/authorize");
}

function tokenEndpoint(c: OidcConfig): string {
  return c.tokenUrl ?? endpoint(c.issuer, "oidc/token");
}

function base64UrlEncode(bytes: Uint8Array): string {
  let s = "";
  for (const b of bytes) s += String.fromCharCode(b);
  return btoa(s).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

/** URL-safe random string (used for the PKCE verifier + state). */
function randomString(bytes = 32): string {
  const buf = new Uint8Array(bytes);
  crypto.getRandomValues(buf);
  return base64UrlEncode(buf);
}

async function sha256(input: string): Promise<Uint8Array> {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(input));
  return new Uint8Array(digest);
}

async function tokenRequest(url: string, body: URLSearchParams): Promise<TokenResponse> {
  const res = await fetch(url, {
    method: "POST",
    headers: { "Content-Type": "application/x-www-form-urlencoded" },
    body,
  });
  if (!res.ok) {
    throw new Error(`OIDC token request failed: ${res.status} ${await res.text().catch(() => "")}`);
  }
  return (await res.json()) as TokenResponse;
}

/**
 * Begin the authorization_code + PKCE flow: redirect the browser to Rauthy's
 * hosted login. Passkeys / MFA / social all happen there. Returns only after
 * navigation has been requested (the page unloads).
 */
export async function loginWithRedirect(c: OidcConfig): Promise<void> {
  const verifier = randomString(48);
  const state = randomString(24);
  const challenge = base64UrlEncode(await sha256(verifier));
  sessionStorage.setItem(PKCE_KEY, JSON.stringify({ verifier, state }));
  const url = new URL(authorizeEndpoint(c));
  url.search = new URLSearchParams({
    response_type: "code",
    client_id: c.clientId,
    redirect_uri: c.redirectUri,
    scope: (c.scopes ?? DEFAULT_SCOPES).join(" "),
    state,
    code_challenge: challenge,
    code_challenge_method: "S256",
  }).toString();
  globalThis.location.assign(url.toString());
}

/** True if the current URL looks like a Rauthy redirect callback (`?code=` or `?error=`). */
export function isRedirectCallback(): boolean {
  const p = new URLSearchParams(globalThis.location.search);
  return p.has("code") || p.has("error");
}

/**
 * Complete the PKCE flow on the callback URL: verify state, exchange the code
 * for a token. Call this on your `redirectUri` route, then hand the
 * `access_token` to your session. Clears the PKCE state from sessionStorage.
 */
export async function handleRedirectCallback(c: OidcConfig): Promise<TokenResponse> {
  const params = new URLSearchParams(globalThis.location.search);
  const error = params.get("error");
  if (error) {
    throw new Error(`OIDC error: ${error} ${params.get("error_description") ?? ""}`.trim());
  }
  const code = params.get("code");
  const returnedState = params.get("state");
  if (!code) throw new Error("OIDC callback missing authorization code");
  const rawPkce = sessionStorage.getItem(PKCE_KEY);
  if (!rawPkce) throw new Error("OIDC PKCE state missing (start the login again)");
  const { verifier, state } = JSON.parse(rawPkce) as { verifier: string; state: string };
  if (state !== returnedState) throw new Error("OIDC state mismatch (possible CSRF)");
  sessionStorage.removeItem(PKCE_KEY);
  return tokenRequest(
    tokenEndpoint(c),
    new URLSearchParams({
      grant_type: "authorization_code",
      code,
      redirect_uri: c.redirectUri,
      client_id: c.clientId,
      code_verifier: verifier,
    }),
  );
}

/**
 * ROPC password grant — the in-app/branded "Kumo-simple" path. Email+password
 * only (no passkeys/MFA). `clientSecret` only for a confidential client; a
 * public SPA client omits it.
 */
export async function loginWithPassword(
  c: OidcConfig,
  username: string,
  password: string,
  clientSecret?: string,
): Promise<TokenResponse> {
  const body = new URLSearchParams({
    grant_type: "password",
    client_id: c.clientId,
    username,
    password,
    scope: (c.scopes ?? DEFAULT_SCOPES).join(" "),
  });
  if (clientSecret) body.set("client_secret", clientSecret);
  return tokenRequest(tokenEndpoint(c), body);
}
