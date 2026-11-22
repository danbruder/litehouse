import { useState } from "react";
import { Link, useNavigate, useParams, useSearchParams } from "react-router-dom";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { ChevronRight, ExternalLink, Play, Plus, RotateCw, Square, Trash2, UploadCloud, X } from "lucide-react";
import { api, type AppDetailSummary, type Deploy } from "../lib/api";
import { relativeTime, absoluteTime } from "../lib/format";
import { useAppAction } from "../lib/actions";
import { Button } from "../components/Button";
import { ConfirmButton } from "../components/ConfirmButton";
import { ErrorNote } from "../components/ErrorNote";
import { ResourceStats } from "../components/ResourceStats";
import { LogTerminal } from "../components/LogTerminal";
import {
  CopyButton,
  Fact,
  Facts,
  Hint,
  Loading,
  Muted,
  Page,
  PageHeader,
  Section,
  StatusDot,
  StatusText,
  Tabs,
} from "../components/ui";

type Tab = "overview" | "deploys" | "logs" | "settings";
const TABS: Tab[] = ["overview", "deploys", "logs", "settings"];

const formRow = "flex flex-col gap-2 sm:flex-row sm:items-center";

function shortSha(sha: string | null) {
  return sha ? sha.slice(0, 7) : null;
}

function CommitLink({ sha, repo }: { sha: string | null; repo: string | null }) {
  if (!sha) return null;
  if (!repo) return <span className="font-mono">{shortSha(sha)}</span>;
  return (
    <a
      href={`https://github.com/${repo}/commit/${sha}`}
      target="_blank"
      rel="noopener noreferrer"
      className="font-mono"
      onClick={(e) => e.stopPropagation()}
    >
      {shortSha(sha)}
    </a>
  );
}

/** One deploy as a tappable row: status, commit, age, and its error if any. */
function DeployRow({ d, app, current }: { d: Deploy; app: AppDetailSummary; current?: boolean }) {
  const navigate = useNavigate();
  const to = `/apps/${encodeURIComponent(app.name)}/deploys/${d.id}`;
  return (
    <li
      className="row-hover flex cursor-pointer items-start gap-3 border-0 border-b border-solid border-rule px-4 py-3 last:border-b-0 md:px-5"
      onClick={() => navigate(to)}
    >
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
          <StatusText state={d.status} />
          {current && (
            <span className="border border-solid border-rule px-1.5 text-[10px] font-semibold uppercase tracking-wider text-ink-3">
              live
            </span>
          )}
          <span className="text-xs text-ink-3" title={absoluteTime(d.created_at)}>
            <CommitLink sha={d.git_sha} repo={app.repo} />
            {d.git_sha && " · "}
            {relativeTime(d.created_at)}
          </span>
        </div>
        <Link to={to} className="row-link mt-1 block truncate text-xs text-ink-3" title={d.image}>
          {d.image}
        </Link>
        {d.error && <p className="m-0 mt-1.5 line-clamp-3 break-words text-xs text-bad">{d.error}</p>}
      </div>
      <ChevronRight size={16} className="mt-0.5 shrink-0 text-ink-3" />
    </li>
  );
}

function ActionBar({ app }: { app: AppDetailSummary }) {
  const qc = useQueryClient();
  const start = useAppAction("start");
  const stop = useAppAction("stop");
  const restart = useAppAction("restart");
  const busy = start.isPending || stop.isPending || restart.isPending;
  const isRunning = app.state === "running";

  const redeploy = useMutation({
    mutationFn: () => {
      if (!app.image) throw new Error("This app has no deployed image yet. Push to its repo first.");
      return api.redeploy(app.name, app.image);
    },
    onSuccess: (result) => {
      if (result.status === "succeeded") toast.success(`${app.name} redeployed`);
      else toast.error("Redeploy failed", { description: result.error ?? result.status });
      qc.invalidateQueries({ queryKey: ["app-deploys", app.name] });
      qc.invalidateQueries({ queryKey: ["app-summary", app.name] });
      qc.invalidateQueries({ queryKey: ["apps-summary"] });
    },
    onError: (err: Error) => toast.error("Redeploy failed", { description: err.message }),
  });

  return (
    <div className="mb-5 grid grid-cols-3 gap-2 md:mb-6 md:flex md:flex-wrap">
      <Button
        size="md"
        variant="solid"
        disabled={busy}
        onClick={() => restart.mutate(app.name)}
      >
        <RotateCw size={14} className={restart.isPending ? "animate-spin" : undefined} /> restart
      </Button>
      <Button
        size="md"
        variant="outline"
        disabled={redeploy.isPending || !app.image}
        title={!app.image ? "This app has no deployed image yet. Push to its repo first." : undefined}
        onClick={() => redeploy.mutate()}
      >
        <UploadCloud size={14} /> {redeploy.isPending ? "deploying…" : "redeploy"}
      </Button>
      {isRunning ? (
        <ConfirmButton
          size="md"
          variant="outline"
          confirmLabel="stop"
          disabled={busy}
          onConfirm={() => stop.mutate(app.name)}
        >
          <Square size={14} /> stop
        </ConfirmButton>
      ) : (
        <Button size="md" variant="outline" disabled={busy} onClick={() => start.mutate(app.name)}>
          <Play size={14} /> start
        </Button>
      )}
    </div>
  );
}

function Overview({ app, deploys }: { app: AppDetailSummary; deploys: Deploy[] | undefined }) {
  const { data: metrics, error } = useQuery({
    queryKey: ["app-metrics", app.name],
    queryFn: () => api.appMetrics(app.name, 24),
    refetchInterval: 30_000,
  });
  const latest = deploys?.[0];

  return (
    <>
      <Section title="Latest deploy" flush>
        {!deploys ? (
          <Muted className="px-4 py-4">loading…</Muted>
        ) : !latest ? (
          <Muted className="px-4 py-4">No deploys yet. Push to the repo to deploy.</Muted>
        ) : (
          <ul className="m-0 list-none p-0">
            <DeployRow d={latest} app={app} />
          </ul>
        )}
      </Section>

      <Section title="Details">
        <Facts>
          <Fact label="URL">
            <span className="flex items-center gap-1">
              <a href={app.url} target="_blank" rel="noopener noreferrer" className="min-w-0 break-all">
                {app.url.replace(/^https?:\/\//, "")}
              </a>
              <CopyButton value={app.url} label="URL" />
            </span>
          </Fact>
          {app.custom_domains.length > 0 && (
            <Fact label="Domains">
              <span className="flex flex-col gap-1">
                {app.custom_domains.map((d) => (
                  <a key={d} href={`https://${d}`} target="_blank" rel="noopener noreferrer" className="break-all">
                    {d}
                  </a>
                ))}
              </span>
            </Fact>
          )}
          {app.repo && (
            <Fact label="Repo">
              <a href={`https://github.com/${app.repo}`} target="_blank" rel="noopener noreferrer">
                {app.repo}
              </a>
            </Fact>
          )}
          {app.image && (
            <Fact label="Image">
              <span className="flex items-start gap-1">
                <code className="min-w-0 break-all !border-0 !bg-transparent !p-0 text-xs text-ink-2">{app.image}</code>
                <CopyButton value={app.image} label="image" className="-mt-1.5 md:-mt-1" />
              </span>
            </Fact>
          )}
          {app.port != null && <Fact label="Port">{app.port}</Fact>}
        </Facts>
      </Section>

      <Section title="Resources · 24h" flush>
        {error ? (
          <div className="px-4 py-3">
            <ErrorNote error={error} what="metrics" />
          </div>
        ) : (
          <ResourceStats samples={metrics ?? []} diskLabel="Data" />
        )}
      </Section>
    </>
  );
}

function EnvSection({ name }: { name: string }) {
  const qc = useQueryClient();
  // Only ever renders `.key` — a saved value never reaches the DOM again.
  const { data: envVars, error, refetch } = useQuery({
    queryKey: ["app-env", name],
    queryFn: () => api.envVars(name),
  });
  const [adding, setAdding] = useState(false);
  const [newKey, setNewKey] = useState("");
  const [newValue, setNewValue] = useState("");

  const setEnv = useMutation({
    mutationFn: () => api.setEnv(name, newKey.trim(), newValue),
    onSuccess: () => {
      toast.success(`Saved ${newKey.trim()}`, { description: "Applies on next start, restart or redeploy." });
      setNewKey("");
      setNewValue("");
      setAdding(false);
      qc.invalidateQueries({ queryKey: ["app-env", name] });
    },
    onError: (err: Error) => toast.error("Failed to save variable", { description: err.message }),
  });
  const deleteEnv = useMutation({
    mutationFn: (key: string) => api.deleteEnv(name, key),
    onSuccess: (_data, key) => {
      toast.success(`Deleted ${key}`);
      qc.invalidateQueries({ queryKey: ["app-env", name] });
    },
    onError: (err: Error) => toast.error("Failed to delete variable", { description: err.message }),
  });

  return (
    <Section
      title="Environment"
      flush
      action={
        !adding && (
          <Button variant="ghost" onClick={() => setAdding(true)}>
            <Plus size={13} /> add
          </Button>
        )
      }
    >
      {error ? (
        <div className="px-4 py-3">
          <ErrorNote error={error} what="environment" onRetry={() => refetch()} />
        </div>
      ) : !envVars || envVars.length === 0 ? (
        !adding && <Muted className="px-4 py-4">No environment variables set.</Muted>
      ) : (
        <ul className="m-0 list-none p-0">
          {envVars.map((e) => (
            <li
              key={e.key}
              className="flex min-h-[48px] items-center justify-between gap-3 border-0 border-b border-solid border-rule px-4 last:border-b-0 md:px-5"
            >
              <span className="min-w-0 break-all font-mono text-sm text-ink">{e.key}</span>
              <span className="flex shrink-0 items-center gap-2">
                <span className="hidden font-mono text-xs tracking-widest text-ink-3 sm:inline">••••••</span>
                <ConfirmButton
                  variant="ghost"
                  confirmLabel="delete"
                  disabled={deleteEnv.isPending}
                  onConfirm={() => deleteEnv.mutate(e.key)}
                  aria-label={`Delete ${e.key}`}
                >
                  <Trash2 size={13} />
                </ConfirmButton>
              </span>
            </li>
          ))}
        </ul>
      )}
      {adding && (
        <form
          className="border-0 border-t border-solid border-rule px-4 py-4 md:px-5"
          onSubmit={(e) => {
            e.preventDefault();
            if (newKey.trim()) setEnv.mutate();
          }}
        >
          <div className={formRow}>
            <input
              type="text"
              placeholder="KEY"
              required
              autoFocus
              autoComplete="off"
              autoCapitalize="characters"
              spellCheck={false}
              value={newKey}
              onChange={(e) => setNewKey(e.target.value)}
              className="font-mono sm:!w-56"
            />
            <input
              type="password"
              placeholder="value"
              required
              autoComplete="off"
              value={newValue}
              onChange={(e) => setNewValue(e.target.value)}
              className="sm:flex-1"
            />
            <div className="grid grid-cols-2 gap-2 sm:flex">
              <Button size="md" variant="solid" type="submit" disabled={setEnv.isPending}>
                save
              </Button>
              <Button size="md" variant="ghost" onClick={() => setAdding(false)}>
                cancel
              </Button>
            </div>
          </div>
          <Hint>Values are write-only. Changes apply on the next start, restart or redeploy.</Hint>
        </form>
      )}
    </Section>
  );
}

function DomainsSection({ name }: { name: string }) {
  const qc = useQueryClient();
  const { data: domains, error, refetch } = useQuery({
    queryKey: ["app-domains", name],
    queryFn: () => api.domains(name),
  });
  const [adding, setAdding] = useState("");

  const invalidate = () => {
    qc.invalidateQueries({ queryKey: ["app-domains", name] });
    qc.invalidateQueries({ queryKey: ["app-summary", name] });
  };
  const add = useMutation({
    mutationFn: () => api.addDomain(name, adding.trim()),
    onSuccess: () => {
      toast.success(`Added ${adding.trim()}`, { description: "Caddy will issue a certificate on first hit." });
      setAdding("");
      invalidate();
    },
    onError: (err: Error) => toast.error("Couldn't add domain", { description: err.message }),
  });
  const remove = useMutation({
    mutationFn: (domain: string) => api.removeDomain(name, domain),
    onSuccess: (_d, domain) => {
      toast.success(`Removed ${domain}`);
      invalidate();
    },
    onError: (err: Error) => toast.error("Couldn't remove domain", { description: err.message }),
  });

  return (
    <Section title="Custom domains" flush>
      {error ? (
        <div className="px-4 py-3">
          <ErrorNote error={error} what="domains" onRetry={() => refetch()} />
        </div>
      ) : (
        domains &&
        domains.length > 0 && (
          <ul className="m-0 list-none p-0">
            {domains.map((d) => (
              <li
                key={d}
                className="flex min-h-[48px] items-center justify-between gap-3 border-0 border-b border-solid border-rule px-4 md:px-5"
              >
                <a
                  href={`https://${d}`}
                  target="_blank"
                  rel="noopener noreferrer"
                  className="inline-flex min-w-0 items-center gap-1.5 break-all text-sm"
                >
                  {d} <ExternalLink size={12} className="shrink-0" />
                </a>
                <ConfirmButton
                  variant="ghost"
                  confirmLabel="remove"
                  disabled={remove.isPending}
                  onConfirm={() => remove.mutate(d)}
                  aria-label={`Remove ${d}`}
                >
                  <X size={14} />
                </ConfirmButton>
              </li>
            ))}
          </ul>
        )
      )}
      <form
        className="px-4 py-4 md:px-5"
        onSubmit={(e) => {
          e.preventDefault();
          if (adding.trim()) add.mutate();
        }}
      >
        <div className={formRow}>
          <input
            type="text"
            inputMode="url"
            placeholder="app.example.com"
            autoComplete="off"
            autoCapitalize="none"
            spellCheck={false}
            value={adding}
            onChange={(e) => setAdding(e.target.value)}
            className="sm:flex-1"
          />
          <Button size="md" variant="outline" type="submit" disabled={add.isPending || !adding.trim()}>
            add domain
          </Button>
        </div>
        <Hint>Point the domain's DNS at this server first. Caddy issues the certificate on the first request.</Hint>
      </form>
    </Section>
  );
}

// Whether a deploy is gated on a readiness path or swapped blind.
function HealthCheckSection({ name }: { name: string }) {
  const qc = useQueryClient();
  const { data: path, error, refetch } = useQuery({
    queryKey: ["app-health-check", name],
    queryFn: () => api.healthCheck(name),
  });
  const [draft, setDraft] = useState("");

  const invalidate = () => qc.invalidateQueries({ queryKey: ["app-health-check", name] });
  const set = useMutation({
    mutationFn: () => api.setHealthCheck(name, draft.trim()),
    onSuccess: () => {
      toast.success(`Health check set to ${draft.trim()}`);
      setDraft("");
      invalidate();
    },
    onError: (err: Error) => toast.error("Couldn't set health check", { description: err.message }),
  });
  const unset = useMutation({
    mutationFn: () => api.unsetHealthCheck(name),
    onSuccess: () => {
      toast.success("Health check cleared");
      invalidate();
    },
    onError: (err: Error) => toast.error("Couldn't clear health check", { description: err.message }),
  });

  return (
    <Section
      title="Health check"
      action={
        path ? (
          <ConfirmButton
            variant="ghost"
            confirmLabel="clear"
            disabled={unset.isPending}
            onConfirm={() => unset.mutate()}
          >
            clear
          </ConfirmButton>
        ) : undefined
      }
    >
      {error ? (
        <ErrorNote error={error} what="health check" onRetry={() => refetch()} />
      ) : path ? (
        <p className="m-0 mb-3 text-sm text-ink-2">
          Deploys wait for <code>{path}</code> to answer before traffic moves over.
        </p>
      ) : (
        <Muted className="mb-3">None. New containers get traffic without being probed first.</Muted>
      )}
      <form
        onSubmit={(e) => {
          e.preventDefault();
          if (draft.trim()) set.mutate();
        }}
      >
        <div className={formRow}>
          <input
            type="text"
            placeholder="/healthz"
            autoComplete="off"
            autoCapitalize="none"
            spellCheck={false}
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            className="font-mono sm:flex-1"
          />
          <Button size="md" variant="outline" type="submit" disabled={set.isPending || !draft.trim()}>
            {path ? "replace" : "set path"}
          </Button>
        </div>
      </form>
    </Section>
  );
}

// Typing the name is deliberate: this removes the app record and its
// container, and the server itself refuses while the app is still running.
function DangerZone({ name }: { name: string }) {
  const navigate = useNavigate();
  const qc = useQueryClient();
  const [typed, setTyped] = useState("");

  const del = useMutation({
    mutationFn: () => api.deleteApp(name),
    onSuccess: () => {
      toast.success(`${name} deleted`);
      qc.invalidateQueries({ queryKey: ["apps-summary"] });
      navigate("/");
    },
    onError: (err: Error) => toast.error(`Couldn't delete ${name}`, { description: err.message }),
  });

  return (
    <Section title="Delete app" tone="danger">
      <Muted className="mb-3">
        Removes the app record, its container and its route. Volumes and S3 backups are left alone. Stop the
        app first: the server refuses to delete a running one.
      </Muted>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          if (typed === name) del.mutate();
        }}
      >
        <div className={formRow}>
          <input
            type="text"
            placeholder={`type “${name}” to confirm`}
            autoComplete="off"
            autoCapitalize="none"
            spellCheck={false}
            value={typed}
            onChange={(e) => setTyped(e.target.value)}
            className="sm:flex-1"
          />
          <Button size="md" variant="danger" type="submit" disabled={typed !== name || del.isPending}>
            <Trash2 size={14} /> delete app
          </Button>
        </div>
      </form>
    </Section>
  );
}

export function AppDetail() {
  const { name = "" } = useParams<{ name: string }>();
  const [params, setParams] = useSearchParams();
  const tabParam = params.get("tab") as Tab | null;
  const tab: Tab = tabParam && TABS.includes(tabParam) ? tabParam : "overview";
  const setTab = (t: Tab) => setParams(t === "overview" ? {} : { tab: t }, { replace: true });

  const {
    data: summary,
    isLoading,
    error: summaryError,
    refetch: refetchSummary,
  } = useQuery({
    queryKey: ["app-summary", name],
    queryFn: () => api.appSummary(name),
    enabled: !!name,
    refetchInterval: 5_000,
  });

  const { data: deploys, error: deploysError } = useQuery({
    queryKey: ["app-deploys", name],
    queryFn: () => api.deploys(name, 20),
    enabled: !!name,
    refetchInterval: 5_000,
  });

  const deploying = deploys?.[0]?.status === "in_progress";
  // A failed deploy leaves the previous container serving, so the live one
  // is the newest that succeeded.
  const liveId = deploys?.find((d) => d.status === "succeeded")?.id;

  return (
    <Page>
      {summaryError ? (
        <>
          <PageHeader title={name} back={{ to: "/", label: "Apps" }} />
          <ErrorNote error={summaryError} what={`app "${name}"`} onRetry={() => refetchSummary()} />
        </>
      ) : isLoading || !summary ? (
        <>
          <PageHeader title={name} back={{ to: "/", label: "Apps" }} />
          <Loading what={name} />
        </>
      ) : (
        <>
          <PageHeader
            back={{ to: "/", label: "Apps" }}
            title={summary.name}
            meta={
              <span className="flex flex-wrap items-center gap-x-3 gap-y-1">
                <StatusText state={summary.state} />
                <a
                  href={summary.url}
                  target="_blank"
                  rel="noopener noreferrer"
                  className="inline-flex min-w-0 items-center gap-1 break-all text-ink-2"
                >
                  {summary.url.replace(/^https?:\/\//, "")} <ExternalLink size={12} className="shrink-0" />
                </a>
              </span>
            }
          />

          <ActionBar app={summary} />

          <Tabs<Tab>
            value={tab}
            onChange={setTab}
            tabs={[
              { id: "overview", label: "Overview" },
              {
                id: "deploys",
                label: "Deploys",
                badge: deploying ? <StatusDot state="in_progress" /> : undefined,
              },
              { id: "logs", label: "Logs" },
              { id: "settings", label: "Settings" },
            ]}
          />

          {tab === "overview" && <Overview app={summary} deploys={deploys} />}

          {tab === "deploys" && (
            <Section title={`Last ${deploys?.length ?? 0} deploys`} flush>
              {deploysError ? (
                <div className="px-4 py-3">
                  <ErrorNote error={deploysError} what="deploys" />
                </div>
              ) : !deploys ? (
                <Muted className="px-4 py-4">loading…</Muted>
              ) : deploys.length === 0 ? (
                <Muted className="px-4 py-4">No deploys yet.</Muted>
              ) : (
                <ul className="m-0 list-none p-0">
                  {deploys.map((d) => (
                    <DeployRow key={d.id} d={d} app={summary} current={d.id === liveId} />
                  ))}
                </ul>
              )}
            </Section>
          )}

          {tab === "logs" && <LogTerminal appName={name} />}

          {tab === "settings" && (
            <>
              <EnvSection name={name} />
              <DomainsSection name={name} />
              <HealthCheckSection name={name} />
              <DangerZone name={name} />
            </>
          )}
        </>
      )}
    </Page>
  );
}
