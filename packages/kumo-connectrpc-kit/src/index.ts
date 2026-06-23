// Public API of @joeblew999/kumo-connectrpc-kit.
// Connect transport + auth:
export { errorMessage, makeTransport, setAuthToken } from "./client.js";
// Rauthy OIDC flows (both: PKCE redirect for passkeys/MFA/social, + password grant):
export {
  loginWithRedirect,
  handleRedirectCallback,
  loginWithPassword,
  isRedirectCallback,
  type OidcConfig,
  type TokenResponse,
} from "./oidc.js";
// Theme helpers:
export { readHtmlTheme, setHtmlTheme, type Theme } from "./theme.js";
// Shared auth/session (Rauthy JWT + whoami) — the one auth for every app:
export {
  AuthProvider,
  useAuth,
  type AuthState,
  type AuthContextValue,
  type AuthProviderProps,
} from "./auth.js";
// Chrome components:
export { PageLoading } from "./PageLoading.js";
export { AuthHero } from "./AuthHero.js";
export { ThemeToggle } from "./ThemeToggle.js";
export { AppShell, type AppShellNavItem, type AppShellProps } from "./AppShell.js";
