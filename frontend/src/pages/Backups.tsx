import { useMemo, useState } from "react";
import { Link } from "react-router-dom";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { api } from "../lib/api";
import { relativeTime, absoluteTime, formatBytes } from "../lib/format";
import { Button } from "../components/Button";
import { ErrorNote } from "../components/ErrorNote";

export function Backups() {
  const qc = useQueryClient();
  const {
    data: backups,
    isLoading,
    error,
    refetch,
  } = useQuery({
    queryKey: ["backups-catalog"],
    queryFn: api.backupCatalog,
    refetchInterval: 30_000,
  });
  const { data: status } = useQuery({
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
      qc.invalidateQueries({ queryKey: ["backups-catalog"] });
      qc.invalidateQueries({ queryKey: ["backup-status"] });
    },
    onError: (err: Error) => toast.error("Backup failed", { description: err.message }),
  });

  const [appFilter, setAppFilter] = useState("all");

  const appNames = useMemo(
    () => Array.from(new Set((backups ?? []).map((b) => b.app_name))).sort(),
    [backups],
  );
  const rows = useMemo(() => {
    const list = appFilter === "all" ? (backups ?? []) : (backups ?? []).filter((b) => b.app_name === appFilter);
    // Newest first: the catalog is most often read to answer "what's the
    // most recent restore point for this app?".
    return [...list].sort((a, b) => Date.parse(b.created_at) - Date.parse(a.created_at));
  }, [backups, appFilter]);
  const totalBytes = rows.reduce((sum, b) => sum + (b.size_bytes ?? 0), 0);

  return (
    <>
      <p>
        <Link to="/">&larr; all apps</Link>
      </p>

      <h2>Backups</h2>

      <div className="card">
        <span className="panel-label">status</span>
        <p className="flex flex-wrap items-center gap-3">
          <span className="muted">
            {status?.last_backup_date
              ? `last clean backup day: ${status.last_backup_date}`
              : "no clean backup day recorded yet"}
          </span>
          <Button variant="outline" disabled={runBackup.isPending} onClick={() => runBackup.mutate()}>
            {runBackup.isPending ? "running…" : "run now"}
          </Button>
        </p>
        <p className="hint">
          A day is only stamped once every app backs up with zero failures. Restores stay a CLI
          operation: <code>lh restore --yes</code>.
        </p>
      </div>

      {error ? (
        <ErrorNote error={error} what="the backup catalog" onRetry={() => refetch()} />
      ) : isLoading ? (
        <p className="muted">loading…</p>
      ) : !backups || backups.length === 0 ? (
        <div className="card">
          <span className="panel-label">catalog</span>
          <p className="muted">No backups recorded yet — the catalog fills in as backups run.</p>
        </div>
      ) : (
        <>
          <div className="flex flex-wrap items-center justify-between gap-3">
            <p className="m-0 text-xs text-ink-3">
              {rows.length} artifact{rows.length === 1 ? "" : "s"} · {formatBytes(totalBytes)}
            </p>
            <label className="flex items-center gap-2 text-xs text-ink-3">
              app
              <select
                value={appFilter}
                onChange={(e) => setAppFilter(e.target.value)}
                className="border border-rule bg-paper px-2 py-1 text-xs text-ink"
              >
                <option value="all">all</option>
                {appNames.map((n) => (
                  <option key={n} value={n}>
                    {n}
                  </option>
                ))}
              </select>
            </label>
          </div>
          <div className="overflow-x-auto">
          <table style={{ minWidth: "34rem" }}>
            <thead>
              <tr>
                <th>App</th>
                <th>Object</th>
                <th>Size</th>
                <th>Age</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((b) => (
                <tr key={b.id}>
                  <td>
                    <Link to={`/apps/${encodeURIComponent(b.app_name)}`}>{b.app_name}</Link>
                  </td>
                  <td className="deploy-image" title={b.s3_key}>
                    <code>{b.s3_key}</code>
                  </td>
                  <td>{formatBytes(b.size_bytes)}</td>
                  <td title={absoluteTime(b.created_at)}>{relativeTime(b.created_at)}</td>
                </tr>
              ))}
            </tbody>
          </table>
          </div>
        </>
      )}
    </>
  );
}
