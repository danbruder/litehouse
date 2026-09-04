import { useState } from "react";
import { Link, useParams } from "react-router-dom";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { Play, Square, RotateCw, UploadCloud } from "lucide-react";
import { api } from "../lib/api";
import { relativeTime, formatBytes } from "../lib/format";
import { StatusBadge, DeployBadge } from "../components/StatusBadge";
import { Button } from "../components/Button";
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

export function AppDetail() {
  const { name = "" } = useParams<{ name: string }>();
  const qc = useQueryClient();

  const { data: summary, isLoading } = useQuery({
    queryKey: ["app-summary", name],
    queryFn: () => api.appSummary(name),
    enabled: !!name,
    refetchInterval: 5_000,
  });

  const { data: deploys } = useQuery({
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

  const [newKey, setNewKey] = useState("");
  const [newValue, setNewValue] = useState("");
  const setEnv = useMutation({
    mutationFn: () => api.setEnv(name, newKey.trim(), newValue),
    onSuccess: () => {
      toast.success(`Saved ${newKey.trim()}`);
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

  const cpu = (metrics ?? []).map((s) => s.cpu_pct);
  const mem = (metrics ?? []).map((s) => s.mem_bytes);
  const disk = (metrics ?? []).map((s) => s.disk_bytes);
  const latestMem = [...mem].reverse().find((v) => v != null) ?? null;
  const latestDisk = [...disk].reverse().find((v) => v != null) ?? null;

  return (
    <>
      <p>
        <Link to="/">&larr; all apps</Link>
      </p>

      {isLoading || !summary ? (
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
              </p>
              {summary.custom_domains.length > 0 && (
                <p>
                  <strong>Domains:</strong>{" "}
                  {summary.custom_domains.map((d) => (
                    <a key={d} href={`https://${d}`} target="_blank" rel="noopener noreferrer">
                      {d}{" "}
                    </a>
                  ))}
                </p>
              )}
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
                </p>
              )}
              {summary.port != null && (
                <p>
                  <strong>Port:</strong> {summary.port}
                </p>
              )}
              <p className="flex flex-wrap gap-2">
                <Button variant="outline" disabled={busy} onClick={() => start.mutate()}>
                  <Play size={12} /> start
                </Button>
                <Button variant="outline" disabled={busy} onClick={() => stop.mutate()}>
                  <Square size={12} /> stop
                </Button>
                <Button variant="outline" disabled={busy} onClick={() => restart.mutate()}>
                  <RotateCw size={12} /> restart
                </Button>
                <Button
                  variant="outline"
                  disabled={redeploy.isPending || !summary.image}
                  title={!summary.image ? "This app has no deployed image yet — push to its repo first." : undefined}
                  onClick={() => redeploy.mutate()}
                >
                  <UploadCloud size={12} /> redeploy
                </Button>
              </p>
            </div>

            <div className="card">
              <span className="panel-label">environment</span>
              {!envVars || envVars.length === 0 ? (
                <p className="muted">no environment variables set</p>
              ) : (
                <ul className="env-list">
                  {envVars.map((e) => (
                    <li key={e.key}>
                      <span className="tag">{e.key}</span>
                      <button
                        type="button"
                        title={`delete ${e.key}`}
                        disabled={deleteEnv.isPending}
                        onClick={() => deleteEnv.mutate(e.key)}
                      >
                        &times;
                      </button>
                    </li>
                  ))}
                </ul>
              )}
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
            </div>
          </div>

          <h3>Resources</h3>
          <div className="metrics-grid">
            <div>
              <h4>
                CPU <span className="muted">24h</span>
              </h4>
              <Sparkline data={cpu} color="var(--color-signal)" height={70} />
            </div>
            <div>
              <h4>
                Memory <span className="muted">of {formatBytes(latestMem)}</span>
              </h4>
              <Sparkline data={mem} color="var(--color-warn)" height={70} />
            </div>
            <div>
              <h4>
                Data size <span className="muted">of {formatBytes(latestDisk)}</span>
              </h4>
              <Sparkline data={disk} color="var(--color-good)" height={70} />
            </div>
          </div>

          <h3>
            Deploys <span className="muted">last {deploys?.length ?? 0}</span>
          </h3>
          <table className="deploys">
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
                    <td>{d.git_sha ? d.git_sha.slice(0, 7) : "-"}</td>
                    <td>{relativeTime(d.created_at)}</td>
                    <td className="deploy-error">
                      {d.error ? <div className="deploy-error-body">{d.error}</div> : <span className="muted">—</span>}
                    </td>
                  </tr>
                ))
              )}
            </tbody>
          </table>

          <h3>Logs</h3>
          <LogTerminal appName={name} />
        </>
      )}
    </>
  );
}
