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
    throw new ApiError(res.status, text || res.statusText);
  }
  if (res.status === 204) return undefined as T;
  const contentType = res.headers.get("content-type") ?? "";
  if (contentType.includes("application/json")) return (await res.json()) as T;
  return (await res.text()) as unknown as T;
}

export const api = {
  appsSummary: () => request<AppSummary[]>("/apps/summary"),
  backupStatus: () => request<BackupStatus>("/backups/status"),
  runBackup: () => request<BackupReport>("/backups/run", { method: "POST" }),
  serverMetrics: (hours = 24) => request<MetricSample[]>(`/metrics/server?hours=${hours}`),
  startApp: (name: string) => request<string>(`/apps/${encodeURIComponent(name)}/start`, { method: "POST" }),
  stopApp: (name: string) => request<string>(`/apps/${encodeURIComponent(name)}/stop`, { method: "POST" }),
  restartApp: (name: string) =>
    request<string>(`/apps/${encodeURIComponent(name)}/restart`, { method: "POST" }),
};
