import { cn } from "../lib/cn";

// Reuses the existing .badge / .badge-{state} classes verbatim (see
// styles.css) so a "running" app reads identically here and on the HTMX
// pages it links out to.
export function StatusBadge({ state, className }: { state: string; className?: string }) {
  return <span className={cn("badge", `badge-${state}`, className)}>{state}</span>;
}

export function DeployBadge({ status, className }: { status: string; className?: string }) {
  return <span className={cn("badge", `badge-deploy-${status}`, className)}>{status}</span>;
}
