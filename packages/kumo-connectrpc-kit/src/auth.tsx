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
  children: ReactNode;
}

export function AuthProvider<W>({
  whoami,
  storageKey = "kit.session",
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
    () => ({ state, setSession: applySession, refreshWhoami, logout }),
    [state, applySession, refreshWhoami, logout],
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
