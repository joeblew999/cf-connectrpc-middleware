import { Text } from "@cloudflare/kumo";
import type { ReactNode } from "react";

/**
 * Shared hero for auth-flow pages (Login, Signup, AcceptInvite):
 * mono eyebrow → display title → secondary lede.
 */
export function AuthHero({
  eyebrow,
  title,
  lede,
}: {
  eyebrow: ReactNode;
  title: ReactNode;
  lede?: ReactNode;
}): ReactNode {
  return (
    <header className="flex flex-col gap-3">
      <p className="font-mono uppercase tracking-wider text-xs text-kumo-subtle m-0">
        {eyebrow}
      </p>
      <h1 className="font-heading text-4xl sm:text-5xl leading-none m-0">{title}</h1>
      {lede && <Text variant="secondary">{lede}</Text>}
    </header>
  );
}
