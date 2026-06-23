import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import type { ReactNode } from "react";
import { setAuthToken } from "./client.js";
import {
  handleRedirectCallback,
  loginWithPassword as oidcLoginWithPassword,
  loginWithRedirect as oidcLoginWithRedirect,
  type OidcConfig,
} from "./oidc.js";

/**
 * The shared auth/session for every ConnectRPC+Kumo app.
 *
 * Pattern: a Rauthy-issued JWT (`token`) + a `whoami` payload, persisted in
 * localStorage. On mount it re-validates the token by calling `whoami` and
 * pushes the token into the kit's Connect transport via `setAuthToken`.
 *
 * Generic over the whoami type `W` — the app supplies the `whoami` call (its
 * codegen'd client against the shared auth proto). The session lifecycle,
 * storage, token-stamping and state machine are owned here, once.
 */
export type AuthState<W> =
  | { status: "loading" }
  | { status: "anonymous" }
  | { status: "authenticated"; token: string; whoami: W };

export interface AuthContextValue<W> {
  state: AuthState<W>;
  setSession: (token: string, whoami: W) => void;
  refreshWhoami: () => Promise<void>;
  logout: () => void;
  /**
   * Begin authorization_code + PKCE — redirect to Rauthy's hosted login. The
   * ONLY flow that supports passkeys / MFA / social (they need the interactive
   * page). Requires `oidc` on the provider. Resolves as the page navigates away.
   */
  loginWithRedirect: () => Promise<void>;
  /**
   * ROPC password grant — the in-app/branded path (email+password only, no
   * passkeys/MFA). On success the session is set (whoami fetched). Requires `oidc`.
   */
  loginWithPassword: (username: string, password: string) => Promise<void>;
  /**
   * Complete the PKCE flow on your redirect-callback route: exchange the code,
   * fetch whoami, set the session. Requires `oidc`.
   */
  completeRedirect: () => Promise<void>;
}

interface Stored<W> {
  token: string;
  whoami: W;
}

const AuthContext = createContext<AuthContextValue<unknown> | null>(null);

export interface AuthProviderProps<W> {
  /** Re-validate + fetch the current identity (the app's codegen'd whoami). */
  whoami: () => Promise<W | null>;
  /** localStorage key for the persisted session. */
  storageKey?: string;
  /**
   * Rauthy OIDC config. When set, enables `loginWithRedirect` (PKCE — passkeys/
   * MFA/social) + `loginWithPassword` + `completeRedirect`. Omit it if your app
   * obtains the token elsewhere and only uses `setSession`.
   */
  oidc?: OidcConfig;
  /** Client secret for the password grant when using a confidential client (a public SPA client omits it). */
  passwordClientSecret?: string;
  children: ReactNode;
}

export function AuthProvider<W>({
  whoami,
  storageKey = "kit.session",
  oidc,
  passwordClientSecret,
  children,
}: AuthProviderProps<W>): ReactNode {
  const [state, setState] = useState<AuthState<W>>({ status: "loading" });
  const tokenRef = useRef<string | null>(null);

  const readStored = useCallback((): Stored<W> | null => {
    try {
      const raw = localStorage.getItem(storageKey);
      if (!raw) return null;
      const parsed = JSON.parse(raw) as Stored<W>;
      return parsed?.token && parsed?.whoami ? parsed : null;
    } catch {
      return null;
    }
  }, [storageKey]);

  const writeStored = useCallback(
    (value: Stored<W> | null): void => {
      if (value) localStorage.setItem(storageKey, JSON.stringify(value));
      else localStorage.removeItem(storageKey);
      setAuthToken(value?.token ?? null);
    },
    [storageKey],
  );

  const applySession = useCallback(
    (token: string, who: W): void => {
      tokenRef.current = token;
      writeStored({ token, whoami: who });
      setState({ status: "authenticated", token, whoami: who });
    },
    [writeStored],
  );

  const logout = useCallback((): void => {
    tokenRef.current = null;
    writeStored(null);
    setState({ status: "anonymous" });
  }, [writeStored]);

  const refreshWhoami = useCallback(async (): Promise<void> => {
    const t = tokenRef.current;
    if (!t) return;
    const who = await whoami();
    if (who) applySession(t, who);
  }, [whoami, applySession]);

  // After a token arrives (any flow): stamp it, fetch whoami, set the session.
  const finishLogin = useCallback(
    async (token: string): Promise<void> => {
      setAuthToken(token);
      const who = await whoami();
      if (!who) {
        setAuthToken(tokenRef.current);
        throw new Error("login succeeded but whoami failed");
      }
      applySession(token, who);
    },
    [whoami, applySession],
  );

  const requireOidc = useCallback((): OidcConfig => {
    if (!oidc) {
      throw new Error("AuthProvider: pass `oidc` to use loginWithRedirect/loginWithPassword");
    }
    return oidc;
  }, [oidc]);

  const loginWithRedirect = useCallback(
    (): Promise<void> => oidcLoginWithRedirect(requireOidc()),
    [requireOidc],
  );

  const loginWithPassword = useCallback(
    async (username: string, password: string): Promise<void> => {
      const tok = await oidcLoginWithPassword(requireOidc(), username, password, passwordClientSecret);
      await finishLogin(tok.access_token);
    },
    [requireOidc, passwordClientSecret, finishLogin],
  );

  const completeRedirect = useCallback(async (): Promise<void> => {
    const tok = await handleRedirectCallback(requireOidc());
    await finishLogin(tok.access_token);
  }, [requireOidc, finishLogin]);

  useEffect(() => {
    const stored = readStored();
    if (!stored) {
      setAuthToken(null);
      setState({ status: "anonymous" });
      return;
    }
    tokenRef.current = stored.token;
    setAuthToken(stored.token);
    setState({ status: "authenticated", token: stored.token, whoami: stored.whoami });
    let cancelled = false;
    whoami()
      .then((who) => {
        if (!cancelled && who) applySession(stored.token, who);
      })
      .catch(() => {
        if (!cancelled) logout();
      });
    return () => {
      cancelled = true;
    };
  }, [readStored, whoami, applySession, logout]);

  const value = useMemo<AuthContextValue<W>>(
    () => ({
      state,
      setSession: applySession,
      refreshWhoami,
      logout,
      loginWithRedirect,
      loginWithPassword,
      completeRedirect,
    }),
    [state, applySession, refreshWhoami, logout, loginWithRedirect, loginWithPassword, completeRedirect],
  );

  return (
    <AuthContext.Provider value={value as AuthContextValue<unknown>}>
      {children}
    </AuthContext.Provider>
  );
}

/** Access the shared auth session. Must be inside an {@link AuthProvider}. */
export function useAuth<W = unknown>(): AuthContextValue<W> {
  const ctx = useContext(AuthContext);
  if (!ctx) throw new Error("useAuth must be used inside <AuthProvider>");
  return ctx as AuthContextValue<W>;
}
