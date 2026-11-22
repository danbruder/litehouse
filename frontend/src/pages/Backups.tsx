import { useMemo, useState } from "react";
import { Link } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";
import { api } from "../lib/api";
import { relativeTime, absoluteTime, formatBytes } from "../lib/format";
import { useRunBackup } from "../lib/actions";
import { Button } from "../components/Button";
import { ErrorNote } from "../components/ErrorNote";
import { Hint, Loading, Muted, Page, PageHeader, Section, StatusDot } from "../components/ui";

export function Backups() {
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
  const runBackup = useRunBackup();
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

  const report = status?.last_backup_report;
  const failed = report?.failed ?? [];

  return (
    <Page>
      <PageHeader
        title="Backups"
        meta="Daily snapshots of every app's /data to S3, kept for 14 days."
        actions={
          <Button
            variant="solid"
            size="md"
            disabled={runBackup.isPending}
            onClick={() => runBackup.mutate()}
          >
            {runBackup.isPending ? "running…" : "run backup now"}
          </Button>
        }
      />

      <Section title="Status">
        <div className="grid grid-cols-2 gap-4">
          <div>
            <div className="text-[11px] font-semibold uppercase tracking-[0.12em] text-ink-3">Last clean day</div>
            <div className="mt-1 font-display text-xl font-bold">{status?.last_backup_date ?? "—"}</div>
          </div>
          <div>
            <div className="text-[11px] font-semibold uppercase tracking-[0.12em] text-ink-3">Last run</div>
            <div className="mt-1 flex items-center gap-2 font-display text-xl font-bold" title={absoluteTime(report?.ran_at)}>
              {report ? (
                <>
                  <StatusDot state={failed.length ? "failed" : "succeeded"} />
                  {relativeTime(report.ran_at)}
                </>
              ) : (
                "—"
              )}
            </div>
            {report && (
              <div className="text-xs text-ink-3">
                {report.succeeded.length} ok{failed.length > 0 && <span className="text-bad"> · {failed.length} failed</span>}
              </div>
            )}
          </div>
        </div>
        {failed.length > 0 && (
          <ul className="m-0 mt-4 list-none space-y-1 border-0 border-t border-solid border-rule p-0 pt-3 text-xs">
            {failed.map(([name, err]) => (
              <li key={name} className="break-words text-ink-3">
                <Link to={`/apps/${encodeURIComponent(name)}`} className="font-semibold text-ink">
                  {name}
                </Link>
                : {err}
              </li>
            ))}
          </ul>
        )}
        <Hint>
          A day is only stamped once every app backs up with zero failures. Restores stay a CLI operation:{" "}
          <code>lh restore --yes</code>.
        </Hint>
      </Section>

      {error ? (
        <ErrorNote error={error} what="the backup catalog" onRetry={() => refetch()} />
      ) : isLoading ? (
        <Loading what="backups" />
      ) : !backups || backups.length === 0 ? (
        <Section title="Catalog">
          <Muted>No backups recorded yet. The catalog fills in as backups run.</Muted>
        </Section>
      ) : (
        <Section
          flush
          title={`${rows.length} file${rows.length === 1 ? "" : "s"} · ${formatBytes(totalBytes)}`}
          action={
            <select value={appFilter} onChange={(e) => setAppFilter(e.target.value)} aria-label="Filter by app">
              <option value="all">All apps</option>
              {appNames.map((n) => (
                <option key={n} value={n}>
                  {n}
                </option>
              ))}
            </select>
          }
        >
          <ul className="m-0 list-none p-0">
            {rows.map((b) => (
              <li
                key={b.id}
                className="border-0 border-b border-solid border-rule px-4 py-3 last:border-b-0 md:grid md:grid-cols-[10rem_minmax(0,1fr)_6rem_5rem] md:items-center md:gap-4 md:px-5"
              >
                <div className="flex items-baseline justify-between gap-3 md:contents">
                  <Link to={`/apps/${encodeURIComponent(b.app_name)}`} className="font-semibold">
                    {b.app_name}
                  </Link>
                  <span className="text-xs text-ink-3 md:order-last md:text-right" title={absoluteTime(b.created_at)}>
                    {relativeTime(b.created_at)}
                  </span>
                </div>
                <div className="mt-0.5 truncate font-mono text-xs text-ink-3 md:mt-0" title={b.s3_key}>
                  {b.s3_key}
                </div>
                <div className="hidden text-right text-xs text-ink-2 md:block">{formatBytes(b.size_bytes)}</div>
                <div className="mt-0.5 text-xs text-ink-2 md:hidden">{formatBytes(b.size_bytes)}</div>
              </li>
            ))}
          </ul>
        </Section>
      )}
    </Page>
  );
}
