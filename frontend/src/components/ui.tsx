import type { ReactNode } from "react";
import { Link } from "react-router-dom";
import { toast } from "sonner";
import { ChevronLeft, Copy } from "lucide-react";
import { cn } from "../lib/cn";

// Small layout primitives shared by every page. Hairline boxes, no radius,
// no shadow — the "Ink & Lime" language from styles.css, but with tighter
// padding on phones than the server-rendered `.card` (which spends 28px a
// side of a 390px screen on whitespace).

export function Page({ children }: { children: ReactNode }) {
  return <div className="mx-auto w-full max-w-5xl px-4 pb-12 pt-4 md:px-8 md:pt-8">{children}</div>;
}

export function PageHeader({
  title,
  back,
  meta,
  actions,
}: {
  title: ReactNode;
  back?: { to: string; label: string };
  meta?: ReactNode;
  actions?: ReactNode;
}) {
  return (
    <div className="mb-5 md:mb-7">
      {back && (
        <Link
          to={back.to}
          className="-ml-1.5 mb-2 inline-flex min-h-[32px] items-center gap-0.5 px-1.5 text-xs font-semibold uppercase tracking-wider text-ink-3"
        >
          <ChevronLeft size={14} /> {back.label}
        </Link>
      )}
      <div className="flex flex-wrap items-end justify-between gap-x-4 gap-y-3">
        <div className="min-w-0">
          <h1 className="m-0 break-words font-display text-[1.75rem] font-bold leading-tight tracking-tight md:text-[2.1rem]">
            {title}
          </h1>
          {meta && <div className="mt-1.5 text-sm text-ink-3">{meta}</div>}
        </div>
        {actions && <div className="flex flex-wrap items-center gap-2">{actions}</div>}
      </div>
    </div>
  );
}

export function Section({
  title,
  action,
  children,
  className,
  tone,
  flush,
}: {
  title?: ReactNode;
  action?: ReactNode;
  children: ReactNode;
  className?: string;
  tone?: "danger";
  /** No inner padding — for lists whose rows run edge to edge. */
  flush?: boolean;
}) {
  return (
    <section
      className={cn(
        "mb-4 border border-solid bg-paper md:mb-5",
        tone === "danger" ? "border-bad" : "border-ink",
        className,
      )}
    >
      {(title || action) && (
        <div
          className={cn(
            "flex min-h-[48px] items-center justify-between gap-3 border-0 border-b border-solid px-4 md:px-5",
            tone === "danger" ? "border-bad" : "border-rule",
          )}
        >
          <h2
            className={cn(
              "m-0 font-sans text-[11px] font-semibold uppercase tracking-[0.14em]",
              tone === "danger" ? "text-bad" : "text-ink-3",
            )}
          >
            {title}
          </h2>
          {action}
        </div>
      )}
      <div className={flush ? "" : "px-4 py-4 md:px-5"}>{children}</div>
    </section>
  );
}

/** Label/value rows: a narrow label column, then a value that may wrap. */
export function Facts({ children }: { children: ReactNode }) {
  return <dl className="m-0 divide-y divide-rule">{children}</dl>;
}

export function Fact({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="grid min-h-[40px] grid-cols-[5.5rem_minmax(0,1fr)] items-baseline gap-3 py-2.5 first:pt-0 last:pb-0 sm:grid-cols-[8rem_minmax(0,1fr)] sm:gap-4">
      <dt className="text-[11px] font-semibold uppercase tracking-[0.1em] text-ink-3">{label}</dt>
      <dd className="m-0 min-w-0 break-words text-sm text-ink">{children}</dd>
    </div>
  );
}

export async function copyText(value: string, label: string) {
  try {
    await navigator.clipboard.writeText(value);
    toast.success(`Copied ${label}`);
  } catch {
    toast.error("Couldn't copy to clipboard");
  }
}

export function CopyButton({ value, label, className }: { value: string; label: string; className?: string }) {
  return (
    <button
      type="button"
      title={`Copy ${label}`}
      aria-label={`Copy ${label}`}
      className={cn(
        "inline-flex h-9 w-9 shrink-0 items-center justify-center border-0 bg-transparent p-0 text-ink-3 hover:bg-paper-3 hover:text-ink md:h-7 md:w-7",
        className,
      )}
      onClick={() => copyText(value, label)}
    >
      <Copy size={14} />
    </button>
  );
}

export function Muted({ children, className }: { children: ReactNode; className?: string }) {
  return <p className={cn("m-0 text-sm text-ink-3", className)}>{children}</p>;
}

export function Hint({ children }: { children: ReactNode }) {
  return <p className="m-0 mt-3 text-xs leading-relaxed text-ink-3">{children}</p>;
}

/** Segmented tabs; scrolls sideways rather than wrapping on narrow screens. */
export function Tabs<T extends string>({
  value,
  onChange,
  tabs,
}: {
  value: T;
  onChange: (v: T) => void;
  tabs: Array<{ id: T; label: string; badge?: ReactNode }>;
}) {
  return (
    <div
      role="tablist"
      className="scrollbar-none -mx-4 mb-4 flex overflow-x-auto border-0 border-b border-solid border-rule px-4 md:mx-0 md:mb-5 md:px-0"
    >
      {tabs.map((t) => {
        const active = t.id === value;
        return (
          <button
            key={t.id}
            type="button"
            role="tab"
            aria-selected={active}
            onClick={() => onChange(t.id)}
            className={cn(
              "-mb-px inline-flex min-h-[44px] shrink-0 items-center gap-1.5 border-0 border-b-2 border-solid bg-transparent px-3 text-xs font-semibold uppercase tracking-wider hover:bg-transparent",
              active
                ? "border-ink text-ink hover:border-ink hover:text-ink"
                : "border-transparent text-ink-3 hover:border-rule hover:text-ink",
            )}
          >
            {t.label}
            {t.badge}
          </button>
        );
      })}
    </div>
  );
}

const STATE_TONE: Record<string, string> = {
  running: "text-good",
  succeeded: "text-good",
  failed: "text-bad",
  crashed: "text-bad",
  starting: "text-warn",
  stopping: "text-warn",
  building: "text-warn",
  in_progress: "text-warn",
};

export function stateTone(state: string) {
  return STATE_TONE[state] ?? "text-ink-3";
}

export function StatusDot({ state, className }: { state: string; className?: string }) {
  const pulsing = ["starting", "stopping", "building", "in_progress"].includes(state);
  return (
    <span
      aria-hidden
      className={cn("dot", stateTone(state), pulsing && "dot-pulse", className)}
    />
  );
}

/** "running" / "in progress" — a status word in its tone, no box. */
export function StatusText({ state, className }: { state: string; className?: string }) {
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1.5 text-[0.7rem] font-bold uppercase tracking-wider",
        stateTone(state),
        className,
      )}
    >
      <StatusDot state={state} />
      {state.replace(/_/g, " ")}
    </span>
  );
}

export function Loading({ what = "" }: { what?: string }) {
  return (
    <div className="flex flex-col gap-3" aria-busy>
      {[0, 1, 2].map((i) => (
        <div key={i} className="h-16 animate-pulse bg-paper-2" style={{ animationDelay: `${i * 120}ms` }} />
      ))}
      <span className="sr-only">loading {what}</span>
    </div>
  );
}
