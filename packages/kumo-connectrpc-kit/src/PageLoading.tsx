import { Loader } from "@cloudflare/kumo";
import type { ReactNode } from "react";

/**
 * Shared loading affordance for in-flight RPC fetches and route guards.
 * Centers a Kumo Loader + optional label.
 */
export function PageLoading({ label }: { label?: string }): ReactNode {
  return (
    <div className="flex flex-col items-center justify-center gap-3 py-12 text-kumo-subtle">
      <Loader size="lg" />
      {label && (
        <span className="font-mono text-sm uppercase tracking-wider">{label}</span>
      )}
    </div>
  );
}
