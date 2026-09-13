import { useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link } from "react-router-dom";
import { toast } from "sonner";
import { Search } from "lucide-react";
import { api } from "../lib/api";
import { relativeTime, absoluteTime, formatBytes } from "../lib/format";
import { AppCard } from "../components/AppCard";
import { Sparkline } from "../components/Sparkline";
import { Button } from "../components/Button";
import { ErrorNote } from "../components/ErrorNote";

function BackupsCard() {
  const qc = useQueryClient();
  const { data, error, refetch } = useQuery({
    queryKey: ["backup-status"],
    queryFn: api.backupStatus,
    refetchInterval: 30_000,
  });
  const runBackup = useMutation({
    mutationFn: api.runBackup,
    onSuccess: (report) => {
      if (report.failed.length > 0) {
        toast.error(`Backup finished with ${report.failed.length} failure(s)`, {
          description: report.failed.map(([name]) => name).join(", "),
        });
      } else {
        toast.success(`Backup finished — ${report.succeeded.length} app(s)`);
      }
      qc.invalidateQueries({ queryKey: ["backup-status"] });
      qc.invalidateQueries({ queryKey: ["backups-catalog"] });
    },
    onError: (err: Error) => toast.error("Backup failed", { description: err.message }),
  });

  const report = data?.last_backup_report;
  const line = report
    ? `${report.succeeded.length} succeeded, ${report.failed.length} failed (last run ${relativeTime(report.ran_at)})`
    : "no backup has run yet";
  // A backup day is only stamped when every app backs up cleanly, so a
  // stale date is the signal worth calling out rather than burying.
  const staleDays = data?.last_backup_date
    ? Math.floor((Date.now() - Date.parse(`${data.last_backup_date}T00:00:00Z`)) / 86_400_000)
    : null;

  return (
    <div className="card">
      <span className="panel-label">backups</span>
      {error ? (
        <ErrorNote error={error} what="backup status" onRetry={() => refetch()} />
      ) : (
        <p className="flex flex-wrap items-center gap-3">
          <span className="muted" title={absoluteTime(report?.ran_at)}>
            {line}
          </span>
          <Link to="/backups">view all</Link>
          <Button variant="outline" disabled={runBackup.isPending} onClick={() => runBackup.mutate()}>
            {runBackup.isPending ? "running…" : "run now"}
          </Button>
        </p>
      )}
      {staleDays != null && staleDays >= 2 && (
        <p className="text-xs text-warn">
          Last clean backup was {staleDays} days ago — check <code>lh backup status</code>.
        </p>
      )}
      {data && data.last_backup_date == null && !error && (
        <p className="hint">
          No clean backup day recorded yet. Configure S3 under <Link to="/settings">settings</Link> if
          you haven't.
        </p>
      )}
      {report && report.failed.length > 0 && (
        <ul>
          {report.failed.map(([name, err]) => (
            <li key={name}>
              <strong>{name}</strong>: <span className="muted">{err}</span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function ServerResourcesCard() {
  const { data, error, refetch } = useQuery({
    queryKey: ["server-metrics"],
    queryFn: () => api.serverMetrics(24),
    refetchInterval: 30_000,
  });
  const samples = data ?? [];
  const series = (pick: (s: (typeof samples)[number]) => number | null) =>
    samples.map((s) => ({ value: pick(s), ts: s.ts }));
  const latest = (pick: (s: (typeof samples)[number]) => number | null) =>
    [...samples].reverse().map(pick).find((v) => v != null) ?? null;
  const latestCpu = latest((s) => s.cpu_pct);
  const latestMem = latest((s) => s.mem_bytes);
  const latestDisk = latest((s) => s.disk_bytes);

  return (
    <div className="card">
      <span className="panel-label">server resources</span>
      {error && <ErrorNote error={error} what="server metrics" onRetry={() => refetch()} />}
      <div className="grid grid-cols-1 gap-6 sm:grid-cols-3">
        <div>
          <h4 className="mb-1 text-xs text-ink-2">
            CPU <span className="muted">{latestCpu != null ? `${latestCpu.toFixed(1)}% · 24h` : "24h"}</span>
          </h4>
          <Sparkline
            data={series((s) => s.cpu_pct)}
            color="var(--color-signal)"
            format={(v) => `${v.toFixed(1)}%`}
          />
        </div>
        <div>
          <h4 className="mb-1 text-xs text-ink-2">
            Memory {latestMem != null && <span className="muted">of {formatBytes(latestMem)}</span>}
          </h4>
          <Sparkline data={series((s) => s.mem_bytes)} color="var(--color-warn)" format={formatBytes} />
        </div>
        <div>
          <h4 className="mb-1 text-xs text-ink-2">
            Disk {latestDisk != null && <span className="muted">of {formatBytes(latestDisk)}</span>}
          </h4>
          <Sparkline data={series((s) => s.disk_bytes)} color="var(--color-good)" format={formatBytes} />
        </div>
      </div>
    </div>
  );
}

export function Dashboard() {
  const {
    data: apps,
    isLoading,
    error,
    refetch,
  } = useQuery({
    queryKey: ["apps-summary"],
    queryFn: api.appsSummary,
    refetchInterval: 5_000,
  });

  const [filter, setFilter] = useState("");
  const visible = useMemo(() => {
    if (!apps) return [];
    const needle = filter.trim().toLowerCase();
    if (!needle) return apps;
    return apps.filter(
      (a) => a.name.toLowerCase().includes(needle) || a.state.toLowerCase().includes(needle),
    );
  }, [apps, filter]);

  // A census across the top: on a host with a dozen apps, "is anything
  // down right now?" shouldn't require reading every card.
  const counts = useMemo(() => {
    const c = { running: 0, stopped: 0, unhealthy: 0 };
    for (const a of apps ?? []) {
      if (a.state === "running") c.running += 1;
      else if (a.state === "failed" || a.state === "crashed") c.unhealthy += 1;
      else c.stopped += 1;
    }
    return c;
  }, [apps]);

  return (
    <>
      <BackupsCard />
      <ServerResourcesCard />

      {error ? (
        <ErrorNote error={error} what="apps" onRetry={() => refetch()} />
      ) : isLoading ? (
        <p className="muted">loading apps…</p>
      ) : !apps || apps.length === 0 ? (
        <div className="card">
          <span className="panel-label">getting started</span>
          <p>No apps yet.</p>
          <p className="muted">
            Run <code>lh create &lt;app&gt; --repo owner/name</code>, then <code>git push</code> to deploy.
          </p>
        </div>
      ) : (
        <>
          <div className="mt-7 flex flex-wrap items-center justify-between gap-3">
            <p className="m-0 flex items-center gap-3 text-xs">
              <span className="badge badge-running">{counts.running} running</span>
              {counts.stopped > 0 && <span className="badge badge-stopped">{counts.stopped} stopped</span>}
              {counts.unhealthy > 0 && (
                <span className="badge badge-failed">{counts.unhealthy} unhealthy</span>
              )}
            </p>
            {apps.length > 3 && (
              <label className="flex items-center gap-2 text-xs text-ink-3">
                <Search size={12} />
                <input
                  type="text"
                  placeholder="filter apps…"
                  value={filter}
                  onChange={(e) => setFilter(e.target.value)}
                  className="text-xs"
                />
              </label>
            )}
          </div>

          {visible.length === 0 ? (
            <p className="muted">no apps match “{filter}”</p>
          ) : (
            <div className="my-4 grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3">
              {visible.map((app) => (
                <AppCard key={app.id} app={app} />
              ))}
            </div>
          )}
        </>
      )}
    </>
  );
}
