import { useState } from "react";
import { Link, useNavigate, useParams } from "react-router-dom";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { Play, Square, RotateCw, UploadCloud, Pencil, Copy, Trash2, ExternalLink } from "lucide-react";
import { api } from "../lib/api";
import { relativeTime, absoluteTime, formatBytes } from "../lib/format";
import { StatusBadge, DeployBadge } from "../components/StatusBadge";
import { Button } from "../components/Button";
import { ConfirmButton } from "../components/ConfirmButton";
import { ErrorNote } from "../components/ErrorNote";
import { Sparkline } from "../components/Sparkline";
import { LogTerminal } from "../components/LogTerminal";

function useAppAction(name: string, action: "start" | "stop" | "restart") {
  const qc = useQueryClient();
  const fn = { start: api.startApp, stop: api.stopApp, restart: api.restartApp }[action];
  return useMutation({
    mutationFn: () => fn(name),
    onSuccess: () => {
      toast.success(`${name} ${action === "stop" ? "stopped" : action === "start" ? "started" : "restarted"}`);
      qc.invalidateQueries({ queryKey: ["app-summary", name] });
      qc.invalidateQueries({ queryKey: ["apps-summary"] });
    },
    onError: (err: Error) => toast.error(`Failed to ${action} ${name}`, { description: err.message }),
  });
}

function CopyButton({ value, label }: { value: string; label: string }) {
  return (
    <button
      type="button"
      title={`copy ${label}`}
      className="ml-1 border-none bg-transparent p-0 align-middle text-ink-3 hover:text-ink"
      onClick={async () => {
        try {
          await navigator.clipboard.writeText(value);
          toast.success(`Copied ${label}`);
        } catch {
          toast.error("Couldn't copy to clipboard");
        }
      }}
    >
      <Copy size={11} />
    </button>
  );
}

// Custom domains were add/remove-able over the API and through `lh domain`,
// but invisible in the browser beyond a read-only list on this page.
function DomainsCard({ name }: { name: string }) {
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
    <div className="card">
      <span className="panel-label">custom domains</span>
      {error ? (
        <ErrorNote error={error} what="domains" onRetry={() => refetch()} />
      ) : !domains || domains.length === 0 ? (
        <p className="muted">none — the app is served on its default subdomain only</p>
      ) : (
        <ul className="env-list">
          {domains.map((d) => (
            <li key={d}>
              <a href={`https://${d}`} target="_blank" rel="noopener noreferrer" className="tag">
                {d} <ExternalLink size={10} />
              </a>
              <ConfirmButton
                variant="ghost"
                confirmLabel={`remove ${d}`}
                disabled={remove.isPending}
                onConfirm={() => remove.mutate(d)}
              >
                &times;
              </ConfirmButton>
            </li>
          ))}
        </ul>
      )}
      <form
        className="env-form"
        onSubmit={(e) => {
          e.preventDefault();
          if (adding.trim()) add.mutate();
        }}
      >
        <input
          type="text"
          placeholder="app.example.com"
          autoComplete="off"
          value={adding}
          onChange={(e) => setAdding(e.target.value)}
        />
        <button type="submit" disabled={add.isPending || !adding.trim()}>
          add
        </button>
      </form>
      <p className="hint">Point the domain's DNS at this server first — Caddy issues the certificate on
        the first request.</p>
    </div>
  );
}

// The health-check endpoints existed since before the SPA but were never
// surfaced anywhere in the UI, so an operator couldn't tell whether a deploy
// was gated on a readiness path or swapped blind.
function HealthCheckCard({ name }: { name: string }) {
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
    <div className="card">
      <span className="panel-label">health check</span>
      {error ? (
        <ErrorNote error={error} what="health check" onRetry={() => refetch()} />
      ) : path ? (
        <p className="flex flex-wrap items-center gap-3">
          <code>{path}</code>
          <ConfirmButton
            variant="ghost"
            confirmLabel="clear health check"
            disabled={unset.isPending}
            onConfirm={() => unset.mutate()}
          >
            clear
          </ConfirmButton>
        </p>
      ) : (
        <p className="muted">none — Caddy routes to the container without probing it first</p>
      )}
      <form
        className="env-form"
        onSubmit={(e) => {
          e.preventDefault();
          if (draft.trim()) set.mutate();
        }}
      >
        <input
          type="text"
          placeholder="/healthz"
          autoComplete="off"
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
        />
        <button type="submit" disabled={set.isPending || !draft.trim()}>
          {path ? "replace" : "set"}
        </button>
      </form>
    </div>
  );
}

// Deleting an app was API- and CLI-only. Typing the name is deliberate: this
// removes the app record and its container, and the server itself refuses
// while the app is still running.
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
    <div className="card" style={{ borderColor: "var(--color-bad)" }}>
      <span className="panel-label" style={{ color: "var(--color-bad)" }}>
        danger zone
      </span>
      <p className="muted">
        Deleting removes the app record, its container and its Caddy route. Volumes and S3 backups are
        left alone. Stop the app first — the server refuses to delete a running one.
      </p>
      <form
        className="env-form"
        onSubmit={(e) => {
          e.preventDefault();
          if (typed === name) del.mutate();
        }}
      >
        <input
          type="text"
          placeholder={`type "${name}" to confirm`}
          autoComplete="off"
          value={typed}
          onChange={(e) => setTyped(e.target.value)}
        />
        <button type="submit" disabled={typed !== name || del.isPending}>
          <Trash2 size={12} /> delete app
        </button>
      </form>
    </div>
  );
}

export function AppDetail() {
  const { name = "" } = useParams<{ name: string }>();
  const qc = useQueryClient();

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
    queryFn: () => api.deploys(name, 8),
    enabled: !!name,
    refetchInterval: 5_000,
  });

  // Only ever renders `.key` below — matches the old HTMX page's guarantee
  // that a saved env var's value never reaches the DOM again.
  const { data: envVars } = useQuery({
    queryKey: ["app-env", name],
    queryFn: () => api.envVars(name),
    enabled: !!name,
  });

  const { data: metrics } = useQuery({
    queryKey: ["app-metrics", name],
    queryFn: () => api.appMetrics(name, 24),
    enabled: !!name,
    refetchInterval: 30_000,
  });

  const start = useAppAction(name, "start");
  const stop = useAppAction(name, "stop");
  const restart = useAppAction(name, "restart");
  const busy = start.isPending || stop.isPending || restart.isPending;

  const redeploy = useMutation({
    mutationFn: () => {
      if (!summary?.image) throw new Error("This app has no deployed image yet — push to its repo first.");
      return api.redeploy(name, summary.image);
    },
    onSuccess: (result) => {
      if (result.status === "succeeded") {
        toast.success(`${name} redeployed`);
      } else {
        toast.error("Redeploy failed", { description: result.error ?? result.status });
      }
      qc.invalidateQueries({ queryKey: ["app-deploys", name] });
      qc.invalidateQueries({ queryKey: ["app-summary", name] });
      qc.invalidateQueries({ queryKey: ["apps-summary"] });
    },
    onError: (err: Error) => toast.error("Redeploy failed", { description: err.message }),
  });

  // The environment card opens read-only (just the saved keys, no controls)
  // — editing is an explicit step so the page at rest never shows a delete
  // button or an open form.
  const [editingEnv, setEditingEnv] = useState(false);
  const [newKey, setNewKey] = useState("");
  const [newValue, setNewValue] = useState("");
  const setEnv = useMutation({
    mutationFn: () => api.setEnv(name, newKey.trim(), newValue),
    onSuccess: () => {
      toast.success(`Saved ${newKey.trim()}`, { description: "Applies on next start, restart or redeploy." });
      setNewKey("");
      setNewValue("");
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

  const samples = metrics ?? [];
  const series = (pick: (s: (typeof samples)[number]) => number | null) =>
    samples.map((s) => ({ value: pick(s), ts: s.ts }));
  const latest = (pick: (s: (typeof samples)[number]) => number | null) =>
    [...samples].reverse().map(pick).find((v) => v != null) ?? null;

  const isRunning = summary?.state === "running";

  return (
    <>
      <p>
        <Link to="/">&larr; all apps</Link>
      </p>

      {summaryError ? (
        <ErrorNote error={summaryError} what={`app "${name}"`} onRetry={() => refetchSummary()} />
      ) : isLoading || !summary ? (
        <p className="muted">loading…</p>
      ) : (
        <>
          <h2>
            {summary.name} <StatusBadge state={summary.state} />
          </h2>

          <div className="detail-grid">
            <div className="card">
              <span className="panel-label">app</span>
              <p>
                <strong>URL:</strong>{" "}
                <a href={summary.url} target="_blank" rel="noopener noreferrer">
                  {summary.url}
                </a>
                <CopyButton value={summary.url} label="URL" />
              </p>
              {summary.repo && (
                <p>
                  <strong>Repo:</strong>{" "}
                  <a href={`https://github.com/${summary.repo}`} target="_blank" rel="noopener noreferrer">
                    {summary.repo}
                  </a>
                </p>
              )}
              {summary.image && (
                <p>
                  <strong>Image:</strong> <code>{summary.image}</code>
                  <CopyButton value={summary.image} label="image" />
                </p>
              )}
              {summary.port != null && (
                <p>
                  <strong>Port:</strong> {summary.port}
                </p>
              )}
              <p className="flex flex-wrap items-center gap-2">
                {isRunning ? (
                  <ConfirmButton
                    variant="outline"
                    confirmLabel={`stop ${name}`}
                    disabled={busy}
                    onConfirm={() => stop.mutate()}
                  >
                    <Square size={12} /> stop
                  </ConfirmButton>
                ) : (
                  <Button variant="outline" disabled={busy} onClick={() => start.mutate()}>
                    <Play size={12} /> start
                  </Button>
                )}
                <Button variant="outline" disabled={busy} onClick={() => restart.mutate()}>
                  <RotateCw size={12} /> restart
                </Button>
                <Button
                  variant="outline"
                  disabled={redeploy.isPending || !summary.image}
                  title={!summary.image ? "This app has no deployed image yet — push to its repo first." : undefined}
                  onClick={() => redeploy.mutate()}
                >
                  <UploadCloud size={12} /> {redeploy.isPending ? "redeploying…" : "redeploy"}
                </Button>
              </p>
            </div>

            <div className="card">
              <div className="panel-header">
                <span className="panel-label">environment</span>
                <Button variant="ghost" size="sm" onClick={() => setEditingEnv((v) => !v)}>
                  <Pencil size={12} /> {editingEnv ? "done" : "edit"}
                </Button>
              </div>
              {!envVars || envVars.length === 0 ? (
                <p className="muted">no environment variables set</p>
              ) : (
                <ul className="env-list">
                  {envVars.map((e) => (
                    <li key={e.key}>
                      <span className="tag">{e.key}</span>
                      {editingEnv && (
                        <ConfirmButton
                          variant="ghost"
                          confirmLabel={`delete ${e.key}`}
                          disabled={deleteEnv.isPending}
                          onConfirm={() => deleteEnv.mutate(e.key)}
                        >
                          &times;
                        </ConfirmButton>
                      )}
                    </li>
                  ))}
                </ul>
              )}
              {editingEnv && (
                <>
                  <form
                    className="env-form"
                    onSubmit={(e) => {
                      e.preventDefault();
                      if (newKey.trim()) setEnv.mutate();
                    }}
                  >
                    <input
                      type="text"
                      name="key"
                      placeholder="KEY"
                      required
                      autoComplete="off"
                      value={newKey}
                      onChange={(e) => setNewKey(e.target.value)}
                    />
                    <input
                      type="password"
                      name="value"
                      placeholder="value"
                      required
                      autoComplete="off"
                      value={newValue}
                      onChange={(e) => setNewValue(e.target.value)}
                    />
                    <button type="submit" disabled={setEnv.isPending}>
                      set
                    </button>
                  </form>
                  <p className="hint">
                    values are never shown once saved — changes apply on next start, restart, or redeploy
                  </p>
                </>
              )}
            </div>

            <DomainsCard name={name} />
            <HealthCheckCard name={name} />
          </div>

          <h3>Resources</h3>
          <div className="metrics-grid">
            <div>
              <h4>
                CPU{" "}
                <span className="muted">
                  {latest((s) => s.cpu_pct) != null ? `${latest((s) => s.cpu_pct)!.toFixed(1)}% · 24h` : "24h"}
                </span>
              </h4>
              <Sparkline
                data={series((s) => s.cpu_pct)}
                color="var(--color-signal)"
                height={70}
                format={(v) => `${v.toFixed(1)}%`}
              />
            </div>
            <div>
              <h4>
                Memory
                {latest((s) => s.mem_bytes) != null && (
                  <span className="muted"> of {formatBytes(latest((s) => s.mem_bytes))}</span>
                )}
              </h4>
              <Sparkline
                data={series((s) => s.mem_bytes)}
                color="var(--color-warn)"
                height={70}
                format={formatBytes}
              />
            </div>
            <div>
              <h4>
                Data size
                {latest((s) => s.disk_bytes) != null && (
                  <span className="muted"> of {formatBytes(latest((s) => s.disk_bytes))}</span>
                )}
              </h4>
              <Sparkline
                data={series((s) => s.disk_bytes)}
                color="var(--color-good)"
                height={70}
                format={formatBytes}
              />
            </div>
          </div>

          <h3>
            Deploys <span className="muted">last {deploys?.length ?? 0}</span>
          </h3>
          {deploysError && <ErrorNote error={deploysError} what="deploys" />}
          {/* At phone width the five fixed-percentage columns collide; let the
              table keep its proportions and scroll instead of overlapping. */}
          <div className="overflow-x-auto">
          <table className="deploys" style={{ minWidth: "38rem" }}>
            <colgroup>
              <col className="col-status" />
              <col className="col-image" />
              <col className="col-sha" />
              <col className="col-when" />
              <col className="col-error" />
            </colgroup>
            <thead>
              <tr>
                <th>Status</th>
                <th>Image</th>
                <th>SHA</th>
                <th>When</th>
                <th>Error</th>
              </tr>
            </thead>
            <tbody>
              {!deploys || deploys.length === 0 ? (
                <tr>
                  <td colSpan={5} className="muted">
                    no deploys yet
                  </td>
                </tr>
              ) : (
                deploys.map((d) => (
                  <tr key={d.id}>
                    <td>
                      <Link to={`/apps/${encodeURIComponent(name)}/deploys/${d.id}`}>
                        <DeployBadge status={d.status} />
                      </Link>
                    </td>
                    <td className="deploy-image" title={d.image}>
                      <Link to={`/apps/${encodeURIComponent(name)}/deploys/${d.id}`}>{d.image}</Link>
                    </td>
                    <td>
                      {d.git_sha && summary.repo ? (
                        <a
                          href={`https://github.com/${summary.repo}/commit/${d.git_sha}`}
                          target="_blank"
                          rel="noopener noreferrer"
                        >
                          {d.git_sha.slice(0, 7)}
                        </a>
                      ) : (
                        d.git_sha?.slice(0, 7) ?? "-"
                      )}
                    </td>
                    <td title={absoluteTime(d.created_at)}>{relativeTime(d.created_at)}</td>
                    <td className="deploy-error">
                      {d.error ? <div className="deploy-error-body">{d.error}</div> : <span className="muted">—</span>}
                    </td>
                  </tr>
                ))
              )}
            </tbody>
          </table>
          </div>

          <h3>Logs</h3>
          <LogTerminal appName={name} />

          <DangerZone name={name} />
        </>
      )}
    </>
  );
}
