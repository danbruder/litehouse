import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link } from "react-router-dom";
import { toast } from "sonner";
import { api } from "../lib/api";
import { relativeTime, formatBytes } from "../lib/format";
import { AppCard } from "../components/AppCard";
import { Sparkline } from "../components/Sparkline";
import { Button } from "../components/Button";

function BackupsCard() {
  const qc = useQueryClient();
  const { data } = useQuery({ queryKey: ["backup-status"], queryFn: api.backupStatus, refetchInterval: 30_000 });
  const runBackup = useMutation({
    mutationFn: api.runBackup,
    onSuccess: () => {
      toast.success("Backup finished");
      qc.invalidateQueries({ queryKey: ["backup-status"] });
    },
    onError: (err: Error) => toast.error("Backup failed", { description: err.message }),
  });

  const report = data?.last_backup_report;
  const line = report
    ? `${report.succeeded.length} succeeded, ${report.failed.length} failed (last run ${relativeTime(report.ran_at)})`
    : "no backup has run yet";

  return (
    <div className="card">
      <span className="panel-label">backups</span>
      <p className="flex flex-wrap items-center gap-3">
        <span className="muted">{line}</span>
        <Link to="/backups">view all</Link>
        <Button variant="outline" disabled={runBackup.isPending} onClick={() => runBackup.mutate()}>
          {runBackup.isPending ? "running…" : "run now"}
        </Button>
      </p>
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
  const { data } = useQuery({
    queryKey: ["server-metrics"],
    queryFn: () => api.serverMetrics(24),
    refetchInterval: 30_000,
  });
  const samples = data ?? [];
  const cpu = samples.map((s) => s.cpu_pct);
  const mem = samples.map((s) => s.mem_bytes);
  const disk = samples.map((s) => s.disk_bytes);
  const latestMem = [...mem].reverse().find((v) => v != null) ?? null;
  const latestDisk = [...disk].reverse().find((v) => v != null) ?? null;

  return (
    <div className="card">
      <span className="panel-label">server resources</span>
      <div className="grid grid-cols-1 gap-6 sm:grid-cols-3">
        <div>
          <h4 className="mb-1 text-xs text-ink-2">
            CPU <span className="muted">24h</span>
          </h4>
          <Sparkline data={cpu} color="var(--color-signal)" />
        </div>
        <div>
          <h4 className="mb-1 text-xs text-ink-2">
            Memory <span className="muted">of {formatBytes(latestMem)}</span>
          </h4>
          <Sparkline data={mem} color="var(--color-warn)" />
        </div>
        <div>
          <h4 className="mb-1 text-xs text-ink-2">
            Disk <span className="muted">of {formatBytes(latestDisk)}</span>
          </h4>
          <Sparkline data={disk} color="var(--color-good)" />
        </div>
      </div>
    </div>
  );
}

export function Dashboard() {
  const { data: apps, isLoading } = useQuery({
    queryKey: ["apps-summary"],
    queryFn: api.appsSummary,
    refetchInterval: 5_000,
  });

  return (
    <>
      <BackupsCard />
      <ServerResourcesCard />

      {isLoading ? (
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
        <div className="my-7 grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3">
          {apps.map((app) => (
            <AppCard key={app.id} app={app} />
          ))}
        </div>
      )}
    </>
  );
}
