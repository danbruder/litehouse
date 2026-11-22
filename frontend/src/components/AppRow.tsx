import { Link } from "react-router-dom";
import { ChevronRight, ExternalLink, Play, RotateCw, Square } from "lucide-react";
import type { AppSummary } from "../lib/api";
import { useAppAction } from "../lib/actions";
import { relativeTime, absoluteTime } from "../lib/format";
import { Button } from "./Button";
import { ConfirmButton } from "./ConfirmButton";
import { StatusDot, StatusText, stateTone } from "./ui";
import { cn } from "../lib/cn";

// One app per row. On a phone the whole row is the tap target into the app
// (actions live on its page, out of reach of a scrolling thumb) with the
// live site one tap away on the right. With a pointer, start/stop and
// restart sit inline too.
export function AppRow({ app }: { app: AppSummary }) {
  const start = useAppAction("start");
  const stop = useAppAction("stop");
  const restart = useAppAction("restart");
  const busy = start.isPending || stop.isPending || restart.isPending;
  const isRunning = app.state === "running";
  const host = app.url.replace(/^https?:\/\//, "");
  const href = `/apps/${encodeURIComponent(app.name)}`;

  return (
    <li className="row-hover relative flex items-center gap-3 border-0 border-b border-solid border-rule last:border-b-0">
      <Link to={href} className="row-link flex min-w-0 flex-1 items-center gap-3 py-3.5 pl-4 md:pl-5">
        <StatusDot state={app.state} className="mt-[1px]" />
        <div className="min-w-0 flex-1">
          <div className="flex items-baseline gap-2">
            <span className="truncate font-display text-[1.05rem] font-semibold text-ink">{app.name}</span>
            {!isRunning && (
              <span className={cn("text-[10px] font-bold uppercase tracking-wider", stateTone(app.state))}>
                {app.state}
              </span>
            )}
          </div>
          <div className="mt-0.5 flex min-w-0 items-center gap-1.5 text-xs text-ink-3">
            <span className="hidden truncate sm:inline">{host}</span>
            <span className="hidden sm:inline">·</span>
            {app.last_deploy_status ? (
              <span className="inline-flex min-w-0 items-center gap-1.5" title={absoluteTime(app.last_deploy_at)}>
                {app.last_deploy_status !== "succeeded" && (
                  <StatusText state={app.last_deploy_status} className="text-[10px]" />
                )}
                {app.last_deploy_status === "succeeded" && <span>deployed</span>}
                {app.last_deploy_sha && <span className="font-mono">{app.last_deploy_sha.slice(0, 7)}</span>}
                <span className="whitespace-nowrap">{relativeTime(app.last_deploy_at)}</span>
              </span>
            ) : (
              <span>never deployed</span>
            )}
          </div>
        </div>
      </Link>

      <div className="hidden items-center gap-1 pr-2 md:flex">
        {isRunning ? (
          <ConfirmButton
            variant="ghost"
            confirmLabel={`stop ${app.name}`}
            disabled={busy}
            onConfirm={() => stop.mutate(app.name)}
          >
            <Square size={12} /> stop
          </ConfirmButton>
        ) : (
          <Button variant="ghost" disabled={busy} onClick={() => start.mutate(app.name)}>
            <Play size={12} /> start
          </Button>
        )}
        <Button variant="ghost" disabled={busy} onClick={() => restart.mutate(app.name)}>
          <RotateCw size={12} className={restart.isPending ? "animate-spin" : undefined} /> restart
        </Button>
      </div>

      <a
        href={app.url}
        target="_blank"
        rel="noopener noreferrer"
        aria-label={`Open ${host}`}
        title={`Open ${host}`}
        className="row-link mr-1 inline-flex h-11 w-11 shrink-0 items-center justify-center text-ink-3 hover:!bg-paper-3 hover:!text-ink md:mr-2 md:h-9 md:w-9"
      >
        <ExternalLink size={16} />
      </a>
      <ChevronRight size={16} className="pointer-events-none -ml-2 mr-3 hidden shrink-0 text-ink-3 sm:block md:hidden" />
    </li>
  );
}
