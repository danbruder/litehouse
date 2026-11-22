import { useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Link } from "react-router-dom";
import { AlertTriangle, ArrowRight, Search } from "lucide-react";
import { api } from "../lib/api";
import { relativeTime, absoluteTime } from "../lib/format";
import { useRunBackup } from "../lib/actions";
import { AppRow } from "../components/AppRow";
import { ResourceStats } from "../components/ResourceStats";
import { Button } from "../components/Button";
import { ErrorNote } from "../components/ErrorNote";
import { Loading, Muted, Page, PageHeader, Section, StatusDot } from "../components/ui";
import { StatusIcon } from "./Health";

function BackupsSection() {
  const { data, error, refetch } = useQuery({
    queryKey: ["backup-status"],
    queryFn: api.backupStatus,
    refetchInterval: 30_000,
  });
  const runBackup = useRunBackup();

  const report = data?.last_backup_report;
  // A backup day is only stamped when every app backs up cleanly, so a
  // stale date is the signal worth calling out rather than burying.
  const staleDays = data?.last_backup_date
    ? Math.floor((Date.now() - Date.parse(`${data.last_backup_date}T00:00:00Z`)) / 86_400_000)
    : null;
  const failed = report?.failed ?? [];
  const tone = !report ? "created" : failed.length > 0 ? "failed" : "succeeded";

  return (
    <Section
      title="Backups"
      action={
        <Link to="/backups" className="inline-flex items-center gap-1 text-xs font-semibold uppercase tracking-wider text-ink-3">
          all <ArrowRight size={12} />
        </Link>
      }
    >
      {error ? (
        <ErrorNote error={error} what="backup status" onRetry={() => refetch()} />
      ) : (
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div className="min-w-0">
            <div className="flex items-center gap-2 text-sm text-ink">
              <StatusDot state={tone} />
              {report ? (
                <span title={absoluteTime(report.ran_at)}>
                  {report.succeeded.length} backed up
                  {failed.length > 0 && <span className="text-bad">, {failed.length} failed</span>}
                  <span className="text-ink-3"> · {relativeTime(report.ran_at)}</span>
                </span>
              ) : (
                <span className="text-ink-3">No backup has run yet</span>
              )}
            </div>
            {failed.length > 0 && (
              <ul className="m-0 mt-2 list-none space-y-1 p-0 text-xs">
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
          </div>
          <Button
            variant="outline"
            size="md"
            className="w-full sm:w-auto"
            disabled={runBackup.isPending}
            onClick={() => runBackup.mutate()}
          >
            {runBackup.isPending ? "running…" : "run backup now"}
          </Button>
        </div>
      )}
      {staleDays != null && staleDays >= 2 && (
        <p className="m-0 mt-3 text-xs text-warn">
          Last clean backup was {staleDays} days ago. Check <code>lh backup status</code>.
        </p>
      )}
      {data && data.last_backup_date == null && !error && (
        <p className="m-0 mt-3 text-xs text-ink-3">
          No clean backup day recorded yet. Configure S3 under <Link to="/settings">settings</Link> if you
          haven't.
        </p>
      )}
    </Section>
  );
}

// One line from the Health page: green when every check passes, otherwise
// the count and the first few things to fix.
function HealthSection() {
  const { data, error } = useQuery({
    queryKey: ["server-health"],
    queryFn: api.serverHealth,
    refetchInterval: 60_000,
  });
  if (error || !data) return null;
  const { ok, warn, fail, unknown } = data.summary;
  const attention = data.checks.filter((c) => c.status === "fail" || c.status === "warn");
  const overall = fail > 0 ? "fail" : warn > 0 ? "warn" : unknown > 0 ? "unknown" : "ok";
  return (
    <Section
      title="Health"
      action={
        <Link to="/health" className="inline-flex items-center gap-1 text-xs font-semibold uppercase tracking-wider text-ink-3">
          all checks <ArrowRight size={12} />
        </Link>
      }
    >
      <Link to="/health" className="flex items-start gap-2.5 text-sm text-ink no-underline">
        <StatusIcon status={overall} size={18} />
        <span className="min-w-0">
          {attention.length === 0 ? (
            <>
              {ok} of {ok + unknown} checks pass
              {unknown > 0 && <span className="text-ink-3"> · {unknown} not checked yet</span>}
            </>
          ) : (
            <>
              {attention.length} {attention.length === 1 ? "thing needs" : "things need"} attention
              <span className="block text-xs text-ink-3">
                {attention
                  .slice(0, 3)
                  .map((c) => c.title)
                  .join(" · ")}
                {attention.length > 3 && ` · +${attention.length - 3} more`}
              </span>
            </>
          )}
        </span>
      </Link>
    </Section>
  );
}

function ServerSection() {
  const { data, error, refetch } = useQuery({
    queryKey: ["server-metrics"],
    queryFn: () => api.serverMetrics(24),
    refetchInterval: 30_000,
  });
  return (
    <Section title="Server" flush>
      {error ? (
        <div className="px-4 py-3">
          <ErrorNote error={error} what="server metrics" onRetry={() => refetch()} />
        </div>
      ) : (
        <ResourceStats samples={data ?? []} />
      )}
    </Section>
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
    return apps.filter((a) => a.name.toLowerCase().includes(needle) || a.state.toLowerCase().includes(needle));
  }, [apps, filter]);

  const unhealthy = (apps ?? []).filter((a) => a.state === "failed" || a.state === "crashed");
  const running = (apps ?? []).filter((a) => a.state === "running").length;
  const stopped = (apps?.length ?? 0) - running - unhealthy.length;

  return (
    <Page>
      <PageHeader
        title="Apps"
        meta={
          apps && apps.length > 0 ? (
            <span className="flex flex-wrap items-center gap-x-3 gap-y-1">
              <span className="inline-flex items-center gap-1.5">
                <StatusDot state="running" /> {running} running
              </span>
              {stopped > 0 && (
                <span className="inline-flex items-center gap-1.5">
                  <StatusDot state="stopped" /> {stopped} stopped
                </span>
              )}
              {unhealthy.length > 0 && (
                <span className="inline-flex items-center gap-1.5 text-bad">
                  <StatusDot state="failed" /> {unhealthy.length} down
                </span>
              )}
            </span>
          ) : undefined
        }
      />

      {unhealthy.length > 0 && (
        <div className="mb-4 flex items-start gap-3 border border-solid border-bad bg-paper px-4 py-3 text-sm md:mb-5">
          <AlertTriangle size={16} className="mt-0.5 shrink-0 text-bad" />
          <div className="min-w-0">
            {unhealthy.map((a, i) => (
              <span key={a.id}>
                {i > 0 && ", "}
                <Link to={`/apps/${encodeURIComponent(a.name)}`} className="font-semibold">
                  {a.name}
                </Link>
              </span>
            ))}{" "}
            <span className="text-ink-2">{unhealthy.length === 1 ? "is" : "are"} down. Open it for logs and deploys.</span>
          </div>
        </div>
      )}

      {error ? (
        <ErrorNote error={error} what="apps" onRetry={() => refetch()} />
      ) : isLoading ? (
        <Loading what="apps" />
      ) : !apps || apps.length === 0 ? (
        <Section title="Getting started">
          <p className="m-0 text-sm text-ink">No apps yet.</p>
          <Muted className="mt-2">
            Run <code>lh create &lt;app&gt; --repo owner/name</code>, then <code>git push</code> to deploy.
          </Muted>
        </Section>
      ) : (
        <Section
          flush
          title={`${apps.length} app${apps.length === 1 ? "" : "s"}`}
          action={
            apps.length > 4 ? (
              <label className="flex min-w-0 items-center gap-2 text-ink-3">
                <Search size={14} className="shrink-0" />
                <input
                  type="text"
                  placeholder="Filter"
                  aria-label="Filter apps"
                  value={filter}
                  onChange={(e) => setFilter(e.target.value)}
                  className="!w-32 !border-0 !bg-transparent !px-0 !py-1 text-sm sm:!w-48"
                />
              </label>
            ) : undefined
          }
        >
          {visible.length === 0 ? (
            <Muted className="px-4 py-4">No apps match “{filter}”.</Muted>
          ) : (
            <ul className="m-0 list-none p-0">
              {visible.map((app) => (
                <AppRow key={app.id} app={app} />
              ))}
            </ul>
          )}
        </Section>
      )}

      <div className="mt-6 grid grid-cols-1 gap-0 md:mt-8">
        <HealthSection />
        <BackupsSection />
        <ServerSection />
      </div>
    </Page>
  );
}
