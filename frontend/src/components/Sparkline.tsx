import { useId } from "react";
import { AreaChart, Area, ResponsiveContainer, Tooltip, YAxis } from "recharts";

export interface SparkPoint {
  value: number | null;
  ts?: string;
}

export function Sparkline({
  data,
  color = "var(--color-ink-2)",
  height = 48,
  format = (v: number) => v.toFixed(1),
}: {
  data: Array<number | null> | SparkPoint[];
  color?: string;
  height?: number;
  /** How a hovered sample is rendered in the tooltip (bytes vs percent). */
  format?: (value: number) => string;
}) {
  // Every chart on a page used to emit `<linearGradient id="sparkline-fill">`
  // — duplicate ids in one document mean the first definition wins, so the
  // CPU chart's blue fill leaked onto the memory and disk charts. useId gives
  // each instance its own gradient.
  const gradientId = useId().replace(/:/g, "");

  const points = (data as Array<number | null | SparkPoint>).map((d, i) =>
    d != null && typeof d === "object"
      ? { i, v: d.value, ts: d.ts }
      : { i, v: d as number | null, ts: undefined },
  );
  const hasData = points.some((p) => p.v != null);

  if (!hasData) {
    return (
      <div className="flex items-center text-[0.7rem] text-ink-3" style={{ height }}>
        no data yet
      </div>
    );
  }

  return (
    <ResponsiveContainer width="100%" height={height}>
      <AreaChart data={points} margin={{ top: 4, right: 0, bottom: 0, left: 0 }}>
        <YAxis hide domain={[0, "auto"]} />
        <defs>
          <linearGradient id={gradientId} x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stopColor={color} stopOpacity={0.25} />
            <stop offset="100%" stopColor={color} stopOpacity={0} />
          </linearGradient>
        </defs>
        <Tooltip
          cursor={{ stroke: "var(--color-rule)" }}
          isAnimationActive={false}
          contentStyle={{
            background: "var(--color-paper)",
            border: "1px solid var(--color-ink)",
            fontSize: "0.7rem",
            padding: "0.25rem 0.5rem",
          }}
          labelFormatter={() => ""}
          formatter={(value, _name, entry) => {
            const ts = (entry?.payload as { ts?: string } | undefined)?.ts;
            const when = ts ? new Date(ts).toLocaleString() : "";
            const num = typeof value === "number" ? value : null;
            return [num == null ? "no sample" : `${format(num)}${when ? ` · ${when}` : ""}`, ""];
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
      </AreaChart>
    </ResponsiveContainer>
  );
}
