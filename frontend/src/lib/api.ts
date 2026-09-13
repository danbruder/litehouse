// Thin client over litehouse's existing JSON API (src/api.rs), the same
// one the `lh` CLI and MCP server use. The SPA authenticates the same way
// the HTMX admin UI already does — an HttpOnly `litehouse_token` cookie set
// by POST /login — so every call here just needs `credentials: "include"`;
// no token ever touches JS. See `admin_auth_middleware` in src/auth.rs.

export interface AppSummary {
  id: string;
  name: string;
  state: string;
  url: string;
  last_deploy_status: string | null;
  last_deploy_sha: string | null;
  last_deploy_at: string | null;
}

// Rust's `(String, String)` tuple serializes as a 2-element JSON array.
export type BackupFailure = [name: string, error: string];

export interface BackupReport {
  ran_at: string;
  succeeded: string[];
  failed: BackupFailure[];
}

export interface BackupStatus {
  last_backup_date: string | null;
  last_backup_report: BackupReport | null;
}

export interface MetricSample {
  ts: string;
  scope: string;
  cpu_pct: number | null;
  mem_bytes: number | null;
  disk_bytes: number | null;
}

// `GET /api/apps/:name/summary` — SPA-only, mirrors `AppSummary` above but
// for one app's detail page: adds the fields the list view doesn't need
// (image, repo, port, custom domains) and drops the ones that do (latest
// deploy — the detail page has the full deploy list instead).
export interface AppDetailSummary {
  id: string;
  name: string;
  state: string;
  url: string;
  image: string | null;
  repo: string | null;
  port: number | null;
  custom_domains: string[];
}

export interface Deploy {
  id: string;
  image: string;
  git_sha: string | null;
  status: string;
  error: string | null;
  created_at: string;
  updated_at: string;
  // Step-by-step narrative of what the deploy engine did (pulling the image,
  // replacing the container, syncing Caddy, ...) — distinct from the app's
  // own container logs.
  log: string;
}

// Values are never rendered once saved (see AppDetail's env card) even
// though this endpoint returns them — matching the old HTMX page's
// behavior of only ever displaying keys.
export interface EnvVar {
  key: string;
  value: string;
}

export interface DeployResult {
  status: string;
  deploy_id: string;
  error?: string;
}

// `GET /api/server/info` — SPA-only; what the settings page shows about
// the server itself. Nothing secret: the S3/GHCR endpoints stay redacted.
export interface ServerInfo {
  version: string;
  domain: string | null;
  admin_host: string | null;
  docker_version: string | null;
  apps_total: number;
  apps_running: number;
  local_dev: boolean;
}

// `GET /api/config/s3` — already redacted server-side (`S3ConfigRedacted`):
// the secret access key never crosses the API.
export interface S3ConfigRedacted {
  access_key_id: string;
  bucket: string;
  region: string;
  endpoint: string | null;
  path_prefix: string | null;
}

export interface S3ConfigInput {
  access_key_id: string;
  secret_access_key: string;
  bucket: string;
  region: string;
  endpoint?: string | null;
  path_prefix?: string | null;
}

// `token` is redacted to its type prefix (e.g. "github_pat_****").
export interface GhcrConfig {
  configured: boolean;
  token: string | null;
}

export interface BackupCatalogEntry {
  id: string;
  app_name: string;
  s3_key: string;
  size_bytes: number;
  status: string;
  created_at: string;
}

class ApiError extends Error {
  constructor(
    public status: number,
    message: string,
  ) {
    super(message);
    this.name = "ApiError";
  }
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(`/api${path}`, {
    credentials: "include",
    headers: { "Content-Type": "application/json" },
    ...init,
  });
  if (res.status === 401) {
    // The cookie expired or was never set (e.g. a bookmarked /  after
    // sign-out beat the redirect) — bounce to the real login page rather
    // than rendering a dead dashboard.
    window.location.href = "/login";
    throw new ApiError(401, "unauthorized");
  }
  if (!res.ok) {
    const text = await res.text().catch(() => res.statusText);
    // A handful of endpoints (deploy, the deploy hook) return a JSON body
    // with an `error` field even on failure — prefer that over the raw
    // envelope when present.
    let message = text || res.statusText;
    if ((res.headers.get("content-type") ?? "").includes("application/json")) {
      try {
        const body = JSON.parse(text) as { error?: string };
        if (body.error) message = body.error;
      } catch {
        // not actually JSON; fall back to the raw text above
      }
    }
    throw new ApiError(res.status, message);
  }
  if (res.status === 204) return undefined as T;
  const contentType = res.headers.get("content-type") ?? "";
  if (contentType.includes("application/json")) return (await res.json()) as T;
  return (await res.text()) as unknown as T;
}

export const api = {
  appsSummary: () => request<AppSummary[]>("/apps/summary"),
  appSummary: (name: string) => request<AppDetailSummary>(`/apps/${encodeURIComponent(name)}/summary`),
  appMetrics: (name: string, hours = 24) =>
    request<MetricSample[]>(`/apps/${encodeURIComponent(name)}/metrics?hours=${hours}`),
  deploys: (name: string, limit = 8) =>
    request<Deploy[]>(`/apps/${encodeURIComponent(name)}/deploys?limit=${limit}`),
  envVars: (name: string) => request<EnvVar[]>(`/apps/${encodeURIComponent(name)}/env`),
  setEnv: (name: string, key: string, value: string) =>
    request<string>(`/apps/${encodeURIComponent(name)}/env`, {
      method: "POST",
      body: JSON.stringify({ key, value }),
    }),
  deleteEnv: (name: string, key: string) =>
    request<string>(`/apps/${encodeURIComponent(name)}/env`, {
      method: "POST",
      body: JSON.stringify({ key, value: "", delete: true }),
    }),
  redeploy: (name: string, image: string) =>
    request<DeployResult>(`/apps/${encodeURIComponent(name)}/deploy`, {
      method: "POST",
      body: JSON.stringify({ image }),
    }),
  logs: (name: string, lines = 300) =>
    request<string>(`/apps/${encodeURIComponent(name)}/logs?lines=${lines}`),
  backupCatalog: () => request<BackupCatalogEntry[]>("/backups/catalog"),
  backupStatus: () => request<BackupStatus>("/backups/status"),
  runBackup: () => request<BackupReport>("/backups/run", { method: "POST" }),
  serverMetrics: (hours = 24) => request<MetricSample[]>(`/metrics/server?hours=${hours}`),
  domains: (name: string) => request<string[]>(`/apps/${encodeURIComponent(name)}/domains`),
  addDomain: (name: string, domain: string) =>
    request<string>(`/apps/${encodeURIComponent(name)}/domains`, {
      method: "POST",
      body: JSON.stringify({ domain }),
    }),
  removeDomain: (name: string, domain: string) =>
    request<string>(`/apps/${encodeURIComponent(name)}/domains`, {
      method: "DELETE",
      body: JSON.stringify({ domain }),
    }),
  // Returns the configured path, or null when the app has none — Caddy
  // then health-checks nothing and routes to the container unconditionally.
  healthCheck: (name: string) => request<string | null>(`/apps/${encodeURIComponent(name)}/health-check`),
  setHealthCheck: (name: string, path: string) =>
    request<string>(`/apps/${encodeURIComponent(name)}/health-check`, {
      method: "POST",
      body: JSON.stringify({ path }),
    }),
  unsetHealthCheck: (name: string) =>
    request<string>(`/apps/${encodeURIComponent(name)}/health-check`, { method: "DELETE" }),
  deleteApp: (name: string) => request<string>(`/apps/${encodeURIComponent(name)}`, { method: "DELETE" }),
  serverInfo: () => request<ServerInfo>("/server/info"),
  // 404 here means "not configured yet", which is a normal state for a
  // fresh install rather than an error worth surfacing as one.
  s3Config: () =>
    request<S3ConfigRedacted>("/config/s3").catch((err: unknown) => {
      if (err instanceof ApiError && err.status === 404) return null;
      throw err;
    }),
  setS3Config: (config: S3ConfigInput) =>
    request<string>("/config/s3", { method: "POST", body: JSON.stringify(config) }),
  deleteS3Config: () => request<string>("/config/s3", { method: "DELETE" }),
  ghcrConfig: () => request<GhcrConfig>("/config/ghcr"),
  setGhcrConfig: (token: string) =>
    request<string>("/config/ghcr", { method: "POST", body: JSON.stringify({ token }) }),
  deleteGhcrConfig: () => request<string>("/config/ghcr", { method: "DELETE" }),
  startApp: (name: string) => request<string>(`/apps/${encodeURIComponent(name)}/start`, { method: "POST" }),
  stopApp: (name: string) => request<string>(`/apps/${encodeURIComponent(name)}/stop`, { method: "POST" }),
  restartApp: (name: string) =>
    request<string>(`/apps/${encodeURIComponent(name)}/restart`, { method: "POST" }),
};
