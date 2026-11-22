import { useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import { CircleCheck, CircleHelp, CircleX, TriangleAlert, ChevronDown } from "lucide-react";
import { api, type CheckStatus, type HealthCheck, type ServerHealth } from "../lib/api";
import { absoluteTime, relativeTime } from "../lib/format";
import { cn } from "../lib/cn";
import { ErrorNote } from "../components/ErrorNote";
import { CopyButton, Hint, Loading, Page, PageHeader, Section } from "../components/ui";

// The server health & security digest: one checklist, grouped, with a green
// check for each thing that's fine and the fix inline for anything that
// isn't. Data comes from GET /api/server/health (src/server_health.rs).

const TONE: Record<CheckStatus, string> = {
  ok: "text-good",
  warn: "text-warn",
  fail: "text-bad",
  unknown: "text-ink-3",
};

const LABEL: Record<CheckStatus, string> = {
  ok: "ok",
  warn: "warning",
  fail: "failing",
  unknown: "not checked",
};

export function StatusIcon({ status, size = 20 }: { status: CheckStatus; size?: number }) {
  const props = { size, className: cn("shrink-0", TONE[status]), "aria-label": LABEL[status] };
  switch (status) {
    case "ok":
      return <CircleCheck {...props} />;
    case "warn":
      return <TriangleAlert {...props} />;
    case "fail":
      return <CircleX {...props} />;
    default:
      return <CircleHelp {...props} />;
  }
}

function uptime(secs: number | null) {
  if (secs == null) return null;
  const d = Math.floor(secs / 86_400);
  const h = Math.floor((secs % 86_400) / 3600);
  if (d > 0) return `up ${d}d ${h}h`;
  const m = Math.floor((secs % 3600) / 60);
  return h > 0 ? `up ${h}h ${m}m` : `up ${m}m`;
}

// Fixes that are a shell command get a copy button; prose fixes don't.
const COMMANDISH = /^(lh |apt|ufw |systemctl |timedatectl |docker |reboot)/;

function Fix({ fix }: { fix: string }) {
  if (COMMANDISH.test(fix)) {
    // "lh upgrade (or ...)" → copy only the command part.
    const cmd = fix.replace(/\s*\(.*\)\s*\.?$/, "").replace(/\.$/, "");
    return (
      <div className="mt-2 flex items-center gap-1 border border-solid border-rule bg-paper-2 pl-3">
        <code className="min-w-0 flex-1 overflow-x-auto whitespace-nowrap py-2 text-xs !bg-transparent !p-0">{fix}</code>
        <CopyButton value={cmd} label="command" />
      </div>
    );
  }
  return (
    <p className="m-0 mt-2 text-xs leading-relaxed text-ink-2">
      <span className="font-semibold uppercase tracking-wider text-ink-3">Fix </span>
      {fix}
    </p>
  );
}

function CheckRow({ check }: { check: HealthCheck }) {
  const needsAttention = check.status === "warn" || check.status === "fail";
  const body = (
    <div className="flex min-w-0 flex-1 items-start gap-3">
      <span className="pt-px">
        <StatusIcon status={check.status} />
      </span>
      <div className="min-w-0 flex-1">
        <div className="text-sm font-semibold leading-snug text-ink">{check.title}</div>
        <div className={cn("mt-0.5 text-sm leading-snug", needsAttention ? "text-ink-2" : "text-ink-3")}>
          {check.summary}
        </div>
      </div>
    </div>
  );

  // Anything that isn't fine shows its explanation and fix up front. Fine
  // checks keep their detail one tap away so the list stays scannable.
  if (needsAttention || check.status === "unknown") {
    return (
      <li id={`check-${check.id}`} className="border-0 border-b border-solid border-rule px-4 py-3.5 last:border-b-0 md:px-5">
        {body}
        {(check.detail || check.fix) && (
          <div className="ml-8 mt-1">
            {check.detail && <p className="m-0 mt-1 text-xs leading-relaxed text-ink-3">{check.detail}</p>}
            {check.fix && <Fix fix={check.fix} />}
          </div>
        )}
      </li>
    );
  }

  if (!check.detail) {
    return (
      <li id={`check-${check.id}`} className="border-0 border-b border-solid border-rule px-4 py-3.5 last:border-b-0 md:px-5">
        {body}
      </li>
    );
  }

  return (
    <li id={`check-${check.id}`} className="border-0 border-b border-solid border-rule last:border-b-0">
      <details className="group">
        <summary className="flex cursor-pointer list-none items-start gap-2 px-4 py-3.5 hover:bg-paper-2 md:px-5 [&::-webkit-details-marker]:hidden">
          {body}
          <ChevronDown size={16} className="mt-0.5 shrink-0 text-ink-3 transition-transform group-open:rotate-180" />
        </summary>
        <p className="m-0 -mt-1 pb-3.5 pl-12 pr-4 text-xs leading-relaxed text-ink-3 md:pr-5">{check.detail}</p>
      </details>
    </li>
  );
}

function SummaryCard({ data }: { data: ServerHealth }) {
  const { ok, warn, fail, unknown } = data.summary;
  const total = ok + warn + fail + unknown;
  const attention = data.checks.filter((c) => c.status === "fail" || c.status === "warn");
  const overall: CheckStatus = fail > 0 ? "fail" : warn > 0 ? "warn" : unknown > 0 ? "unknown" : "ok";
  const headline =
    overall === "ok"
      ? `All ${total} checks pass`
      : overall === "unknown"
        ? `${ok} of ${total} checks pass`
        : `${attention.length} ${attention.length === 1 ? "thing needs" : "things need"} attention`;

  return (
    <section
      className={cn(
        "mb-4 border border-solid bg-paper px-4 py-4 md:mb-5 md:px-5",
        overall === "fail" ? "border-bad" : "border-ink",
      )}
    >
      <div className="flex items-center gap-3">
        <StatusIcon status={overall} size={28} />
        <div className="min-w-0">
          <div className="font-display text-xl font-bold leading-tight">{headline}</div>
          <div className="mt-1 flex flex-wrap gap-x-3 gap-y-0.5 text-xs text-ink-3">
            <span className="text-good">{ok} ok</span>
            {warn > 0 && <span className="text-warn">{warn} warning{warn === 1 ? "" : "s"}</span>}
            {fail > 0 && <span className="text-bad">{fail} failing</span>}
            {unknown > 0 && <span>{unknown} not checked</span>}
          </div>
        </div>
      </div>
      {attention.length > 0 && (
        <ul className="m-0 mt-4 list-none space-y-2 border-0 border-t border-solid border-rule p-0 pt-3">
          {attention.map((c) => (
            <li key={c.id}>
              <a href={`#check-${c.id}`} className="flex items-center gap-2 text-sm text-ink no-underline">
                <StatusIcon status={c.status} size={16} />
                <span className="min-w-0 truncate">{c.title}</span>
              </a>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

export function Health() {
  const { data, isLoading, error, refetch, isFetching } = useQuery({
    queryKey: ["server-health"],
    queryFn: api.serverHealth,
    refetchInterval: 60_000,
  });

  const groups = useMemo(() => {
    const out: Array<{ name: string; checks: HealthCheck[] }> = [];
    for (const c of data?.checks ?? []) {
      const g = out.find((x) => x.name === c.group);
      if (g) g.checks.push(c);
      else out.push({ name: c.group, checks: [c] });
    }
    return out;
  }, [data]);

  const meta = data
    ? [data.hostname, data.os, uptime(data.uptime_secs)].filter(Boolean).join(" · ") ||
      "Firewall, access, updates, resources and backups."
    : "Firewall, access, updates, resources and backups.";

  return (
    <Page>
      <PageHeader
        title="Health"
        meta={meta}
        actions={
          <button
            type="button"
            onClick={() => refetch()}
            disabled={isFetching}
            className="min-h-[40px] px-3 text-xs font-semibold uppercase tracking-wider"
          >
            {isFetching ? "checking…" : "check now"}
          </button>
        }
      />

      {error ? (
        <ErrorNote error={error} what="server health" onRetry={() => refetch()} />
      ) : isLoading || !data ? (
        <Loading what="health checks" />
      ) : (
        <>
          <SummaryCard data={data} />
          {groups.map((g) => {
            const bad = g.checks.filter((c) => c.status === "warn" || c.status === "fail").length;
            const ok = g.checks.filter((c) => c.status === "ok").length;
            return (
              <Section
                key={g.name}
                flush
                title={g.name}
                action={
                  <span className={cn("text-xs", bad > 0 ? "text-warn" : "text-ink-3")}>
                    {bad > 0 ? `${bad} to fix` : `${ok} of ${g.checks.length} ok`}
                  </span>
                }
              >
                <ul className="m-0 list-none p-0">
                  {g.checks.map((c) => (
                    <CheckRow key={c.id} check={c} />
                  ))}
                </ul>
              </Section>
            );
          })}
          <Hint>
            Server checks run live. Host checks (firewall, ports, SSH, packages, disk, certificates) come from a
            read-only report the host refreshes every 15 minutes
            {data.host_checked_at && (
              <span title={absoluteTime(data.host_checked_at)}>; last one {relativeTime(data.host_checked_at)}</span>
            )}
            . Same list from a terminal: <code>lh server-health</code>.
          </Hint>
        </>
      )}
    </Page>
  );
}
