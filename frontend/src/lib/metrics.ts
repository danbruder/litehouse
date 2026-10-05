import type { MetricSample } from "./api";
import type { SparkPoint } from "../components/Sparkline";

// CPU is sampled once a minute, and a one-minute busy fraction is bursty by
// nature: a box that is 50% loaded looks like a solid band flipping between
// ~100% and ~1%, so the raw line can't tell "busy sometimes" from "pegged".
// Averaging into fixed wall-clock buckets shows the sustained load, and the
// bucket's peak is kept alongside so short spikes aren't hidden either.
export const CPU_BUCKET_MINUTES = 15;

export function bucketCpu(samples: MetricSample[], minutes = CPU_BUCKET_MINUTES): SparkPoint[] {
  const size = minutes * 60_000;
  const buckets = new Map<number, number[]>();
  for (const s of samples) {
    const t = Date.parse(s.ts);
    if (Number.isNaN(t)) continue;
    const key = Math.floor(t / size) * size;
    const vals = buckets.get(key) ?? [];
    if (s.cpu_pct != null) vals.push(s.cpu_pct);
    buckets.set(key, vals);
  }
  return [...buckets.entries()]
    .sort(([a], [b]) => a - b)
    .map(([key, vals]) => ({
      ts: new Date(key).toISOString(),
      value: vals.length ? vals.reduce((a, b) => a + b, 0) / vals.length : null,
      peak: vals.length ? Math.max(...vals) : null,
    }));
}

/** Mean CPU over the trailing `minutes` — the headline number. A single
 *  latest sample is as likely to read 1% as 95% on a bursty box. */
export function recentCpuAvg(samples: MetricSample[], minutes = CPU_BUCKET_MINUTES): number | null {
  const cutoff = Date.now() - minutes * 60_000;
  const vals = samples
    .filter((s) => s.cpu_pct != null && Date.parse(s.ts) >= cutoff)
    .map((s) => s.cpu_pct as number);
  return vals.length ? vals.reduce((a, b) => a + b, 0) / vals.length : null;
}

export function peakCpu(samples: MetricSample[]): number | null {
  const vals = samples.filter((s) => s.cpu_pct != null).map((s) => s.cpu_pct as number);
  return vals.length ? Math.max(...vals) : null;
}

/** "12.3% · 15m avg · 24h peak 98%" — the CPU chart's heading. */
export function cpuHeading(samples: MetricSample[]): string {
  const avg = recentCpuAvg(samples);
  const peak = peakCpu(samples);
  const parts = [avg != null ? `${avg.toFixed(1)}% · ${CPU_BUCKET_MINUTES}m avg` : null, peak != null ? `24h peak ${peak.toFixed(0)}%` : "24h"];
  return parts.filter(Boolean).join(" · ");
}
