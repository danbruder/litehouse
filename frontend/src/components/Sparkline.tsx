import { useId } from "react";
import { ComposedChart, Area, Line, ResponsiveContainer, Tooltip, YAxis } from "recharts";

export interface SparkPoint {
  value: number | null;
  ts?: string;
  /** Highest raw sample within a bucketed point — drawn as a faint line
   *  over the (averaged) area. */
  peak?: number | null;
}

export function Sparkline({
  data,
  color = "var(--color-ink-2)",
  height = 48,
  format = (v: number) => v.toFixed(1),
  max = "auto",
}: {
  data: Array<number | null> | SparkPoint[];
  color?: string;
  height?: number;
  /** How a hovered sample is rendered in the tooltip (bytes vs percent). */
  format?: (value: number) => string;
  /** Fixed top of the y-axis (e.g. 100 for a percentage) instead of
   *  scaling to the data, so a flat 2% line doesn't fill the chart. */
  max?: number | "auto";
}) {
  // Every chart on a page used to emit `<linearGradient id="sparkline-fill">`
  // — duplicate ids in one document mean the first definition wins, so the
  // CPU chart's blue fill leaked onto the memory and disk charts. useId gives
  // each instance its own gradient.
  const gradientId = useId().replace(/:/g, "");

  const points = (data as Array<number | null | SparkPoint>).map((d, i) =>
    d != null && typeof d === "object"
      ? { i, v: d.value, ts: d.ts, p: d.peak ?? null }
      : { i, v: d as number | null, ts: undefined, p: null },
  );
  const hasData = points.some((p) => p.v != null);
  const hasPeak = points.some((p) => p.p != null);

  if (!hasData) {
    return (
      <div className="flex items-center text-[0.7rem] text-ink-3" style={{ height }}>
        no data yet
      </div>
    );
  }

  return (
    <ResponsiveContainer width="100%" height={height}>
      <ComposedChart data={points} margin={{ top: 4, right: 0, bottom: 0, left: 0 }}>
        <YAxis hide domain={[0, max === "auto" ? "auto" : (dataMax: number) => Math.max(max, dataMax)]} />
        <defs>
          <linearGradient id={gradientId} x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stopColor={color} stopOpacity={0.25} />
            <stop offset="100%" stopColor={color} stopOpacity={0} />
          </linearGradient>
        </defs>
        <Tooltip
          cursor={{ stroke: "var(--color-rule)" }}
          isAnimationActive={false}
          content={({ active, payload }) => {
            const pt = active ? (payload?.[0]?.payload as (typeof points)[number] | undefined) : undefined;
            if (!pt) return null;
            const when = pt.ts ? new Date(pt.ts).toLocaleString() : "";
            const val = pt.v == null ? "no sample" : format(pt.v);
            return (
              <div
                style={{
                  background: "var(--color-paper)",
                  border: "1px solid var(--color-ink)",
                  fontSize: "0.7rem",
                  padding: "0.25rem 0.5rem",
                }}
              >
                {hasPeak && pt.v != null ? `avg ${val}` : val}
                {hasPeak && pt.p != null && ` · peak ${format(pt.p)}`}
                {when && <div className="text-ink-3">{when}</div>}
              </div>
            );
          }}
        />
        <Area
          type="monotone"
          dataKey="v"
          stroke={color}
          strokeWidth={1.5}
          fill={`url(#${gradientId})`}
          isAnimationActive={false}
          connectNulls
        />
        {hasPeak && (
          <Line
            type="monotone"
            dataKey="p"
            stroke={color}
            strokeWidth={1}
            strokeOpacity={0.45}
            strokeDasharray="2 2"
            dot={false}
            activeDot={false}
            isAnimationActive={false}
            connectNulls
          />
        )}
      </ComposedChart>
    </ResponsiveContainer>
  );
}
