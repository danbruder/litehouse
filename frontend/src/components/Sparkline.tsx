import { AreaChart, Area, ResponsiveContainer, YAxis } from "recharts";

export function Sparkline({
  data,
  color = "var(--color-ink-2)",
  height = 48,
}: {
  data: Array<number | null>;
  color?: string;
  height?: number;
}) {
  const points = data.map((v, i) => ({ i, v }));
  const hasData = data.some((v) => v != null);

  if (!hasData) {
    return (
      <div
        className="flex items-center text-[0.7rem] text-ink-3"
        style={{ height }}
      >
        no data yet
      </div>
    );
  }

  return (
    <ResponsiveContainer width="100%" height={height}>
      <AreaChart data={points} margin={{ top: 4, right: 0, bottom: 0, left: 0 }}>
        <YAxis hide domain={[0, "auto"]} />
        <defs>
          <linearGradient id="sparkline-fill" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stopColor={color} stopOpacity={0.25} />
            <stop offset="100%" stopColor={color} stopOpacity={0} />
          </linearGradient>
        </defs>
        <Area
          type="monotone"
          dataKey="v"
          stroke={color}
          strokeWidth={1.5}
          fill="url(#sparkline-fill)"
          isAnimationActive={false}
          connectNulls
        />
      </AreaChart>
    </ResponsiveContainer>
  );
}
