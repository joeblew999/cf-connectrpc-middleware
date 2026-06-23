export type Theme = string;

/** Live-mutate the `<html data-theme>` attribute. No persistence. */
export function setHtmlTheme(theme: Theme): void {
  document.documentElement.setAttribute("data-theme", theme);
}

/**
 * Read the current `<html data-theme>`, falling back to the first of `themes`
 * if the attribute is unset or not in the list. The consumer owns its theme
 * list (scenario themes + Kumo built-ins like "kumo"/"fedramp").
 */
export function readHtmlTheme(themes: readonly string[]): Theme {
  const t = document.documentElement.getAttribute("data-theme");
  if (t && themes.includes(t)) return t;
  return themes[0] ?? "kumo";
}
