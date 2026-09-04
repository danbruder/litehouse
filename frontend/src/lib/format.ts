// Mirrors src/ui.rs's `relative_time_at` / `chart::format_bytes` so the SPA
// reads identically to the HTMX pages it sits next to.

export function relativeTime(rfc3339: string | null | undefined): string {
  if (!rfc3339) return "no deploys";
  const ts = Date.parse(rfc3339);
  if (Number.isNaN(ts)) return rfc3339;
  const secs = Math.max(0, Math.floor((Date.now() - ts) / 1000));
  if (secs <= 59) return "just now";
  if (secs < 3600) return `${Math.floor(secs / 60)}m ago`;
  if (secs < 86_400) return `${Math.floor(secs / 3600)}h ago`;
  return `${Math.floor(secs / 86_400)}d ago`;
}

export function formatBytes(bytes: number | null | undefined): string {
  if (bytes == null) return "unknown";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(unit === 0 ? 0 : 1)} ${units[unit]}`;
}
