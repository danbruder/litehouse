import type { MetricSample } from "../lib/api";
import { formatBytes } from "../lib/format";
import { bucketCpu, peakCpu, recentCpuAvg, CPU_BUCKET_MINUTES } from "../lib/metrics";
import { Sparkline } from "./Sparkline";

// CPU / memory / disk as three stat tiles: the number you came for in big
// type, the 24h shape underneath. CPU on its own row on phones (memory and
// disk share the next), all three side by side on wider screens.

function Stat({
  label,
  value,
  sub,
  wide,
  children,
}: {
  label: string;
  value: string;
  sub?: string;
  /** Full row on phones (CPU, whose sub-line needs the room). */
  wide?: boolean;
  children: React.ReactNode;
}) {
  return (
    <div className={`min-w-0 bg-paper px-4 py-3.5 md:px-5 ${wide ? "col-span-2 sm:col-span-1" : ""}`}>
      <div className="flex items-baseline justify-between gap-3">
        <span className="text-[11px] font-semibold uppercase tracking-[0.12em] text-ink-3">{label}</span>
        {sub && <span className="truncate text-[11px] text-ink-3">{sub}</span>}
      </div>
      <div className="mt-0.5 font-display text-2xl font-bold tracking-tight text-ink">{value}</div>
      <div className="mt-1">{children}</div>
    </div>
  );
}

export function ResourceStats({
  samples,
  diskLabel = "Disk",
}: {
  samples: MetricSample[];
  diskLabel?: string;
}) {
  const series = (pick: (s: MetricSample) => number | null) => samples.map((s) => ({ value: pick(s), ts: s.ts }));
  const latest = (pick: (s: MetricSample) => number | null) =>
    [...samples].reverse().map(pick).find((v) => v != null) ?? null;
  const cpu = recentCpuAvg(samples);
  const peak = peakCpu(samples);
  const mem = latest((s) => s.mem_bytes);
  const disk = latest((s) => s.disk_bytes);

  return (
    <div className="grid grid-cols-2 gap-px bg-rule sm:grid-cols-3">
      <Stat
        wide
        label="CPU"
        value={cpu != null ? `${cpu.toFixed(1)}%` : "—"}
        sub={[`${CPU_BUCKET_MINUTES}m avg`, peak != null ? `peak ${peak.toFixed(0)}%` : null].filter(Boolean).join(" · ")}
      >
        <Sparkline data={bucketCpu(samples)} color="var(--color-signal)" max={100} height={40} format={(v) => `${v.toFixed(1)}%`} />
      </Stat>
      <Stat label="Memory" value={mem != null ? formatBytes(mem) : "—"} sub="24h">
        <Sparkline data={series((s) => s.mem_bytes)} color="var(--color-warn)" height={40} format={formatBytes} />
      </Stat>
      <Stat label={diskLabel} value={disk != null ? formatBytes(disk) : "—"} sub="24h">
        <Sparkline data={series((s) => s.disk_bytes)} color="var(--color-good)" height={40} format={formatBytes} />
      </Stat>
    </div>
  );
}
