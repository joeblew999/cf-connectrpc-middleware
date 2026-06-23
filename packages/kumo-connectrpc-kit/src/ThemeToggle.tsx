import { useEffect, useState } from "react";
import type { ReactNode } from "react";
import { readHtmlTheme, setHtmlTheme, type Theme } from "./theme.js";

/**
 * Dev theme A/B tool — mutates `<html data-theme>` live so you can verify
 * components paint under every theme, and restores the canonical theme on
 * unmount. Decoupled from any app: pass your `themes` list as a prop.
 */
export function ThemeToggle({ themes }: { themes: readonly string[] }): ReactNode {
  const [canonical] = useState<Theme>(() => readHtmlTheme(themes));
  const [theme, setTheme] = useState<Theme>(canonical);

  useEffect(() => {
    setHtmlTheme(theme);
  }, [theme]);

  useEffect(() => {
    return () => {
      setHtmlTheme(canonical);
    };
  }, [canonical]);

  return (
    <div
      role="group"
      aria-label="Theme"
      style={{
        display: "inline-flex",
        gap: 2,
        padding: 2,
        border: "1px solid var(--border-visible)",
        borderRadius: "var(--radius-pill)",
        fontFamily: "var(--font-mono)",
        fontSize: 11,
        letterSpacing: "0.08em",
        textTransform: "uppercase",
      }}
    >
      {themes.map((t) => {
        const active = t === theme;
        return (
          <button
            key={t}
            type="button"
            onClick={() => setTheme(t)}
            aria-pressed={active}
            style={{
              all: "unset",
              cursor: "pointer",
              padding: "2px 10px",
              borderRadius: "var(--radius-pill)",
              background: active ? "var(--accent)" : "transparent",
              color: active ? "var(--text-display)" : "var(--text-secondary)",
              minHeight: 0,
            }}
          >
            {t}
          </button>
        );
      })}
    </div>
  );
}
