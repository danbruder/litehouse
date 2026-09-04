import { useMutation, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { Play, Square, RotateCw, ExternalLink } from "lucide-react";
import type { AppSummary } from "../lib/api";
import { api } from "../lib/api";
import { relativeTime } from "../lib/format";
import { Button } from "./Button";
import { StatusBadge, DeployBadge } from "./StatusBadge";

function useAppAction(action: "start" | "stop" | "restart") {
  const qc = useQueryClient();
  const fn = { start: api.startApp, stop: api.stopApp, restart: api.restartApp }[action];
  return useMutation({
    mutationFn: (name: string) => fn(name),
    onSuccess: (_data, name) => {
      toast.success(`${name} ${action === "stop" ? "stopped" : action === "start" ? "started" : "restarted"}`);
      qc.invalidateQueries({ queryKey: ["apps-summary"] });
    },
    onError: (err: Error, name) => {
      toast.error(`Failed to ${action} ${name}`, { description: err.message });
    },
  });
}

export function AppCard({ app }: { app: AppSummary }) {
  const start = useAppAction("start");
  const stop = useAppAction("stop");
  const restart = useAppAction("restart");
  const busy = start.isPending || stop.isPending || restart.isPending;
  const isRunning = app.state === "running";

  return (
    <div className="site-card card !my-0 flex flex-col justify-between">
      <div>
        <div className="flex items-start justify-between gap-3">
          <a
            href={`/apps/${encodeURIComponent(app.name)}`}
            className="font-display text-base font-semibold text-ink hover:text-lime"
          >
            {app.name}
          </a>
          <StatusBadge state={app.state} />
        </div>
        <a
          href={app.url}
          target="_blank"
          rel="noopener noreferrer"
          className="mt-1 inline-flex items-center gap-1 text-xs text-ink-3 hover:text-ink"
        >
          {app.url.replace(/^https?:\/\//, "")}
          <ExternalLink size={11} />
        </a>

        <div className="mt-3 flex items-center gap-2 text-xs text-ink-2">
          {app.last_deploy_status ? (
            <DeployBadge status={app.last_deploy_status} />
          ) : (
            <span className="text-ink-3">no deploys</span>
          )}
          {app.last_deploy_sha && (
            <span className="font-mono text-ink-3">{app.last_deploy_sha}</span>
          )}
          {app.last_deploy_at && (
            <span className="text-ink-3">· {relativeTime(app.last_deploy_at)}</span>
          )}
        </div>
      </div>

      <div className="mt-4 flex gap-2 border-t border-rule pt-3">
        {isRunning ? (
          <Button variant="outline" disabled={busy} onClick={() => stop.mutate(app.name)}>
            <Square size={12} /> Stop
          </Button>
        ) : (
          <Button variant="outline" disabled={busy} onClick={() => start.mutate(app.name)}>
            <Play size={12} /> Start
          </Button>
        )}
        <Button variant="ghost" disabled={busy} onClick={() => restart.mutate(app.name)}>
          <RotateCw size={12} /> Restart
        </Button>
      </div>
    </div>
  );
}
