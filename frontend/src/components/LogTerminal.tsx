import { useEffect, useRef } from "react";
import { useQuery } from "@tanstack/react-query";
import { api } from "../lib/api";

// Polls the same one-shot log endpoint the CLI uses (`GET
// /api/apps/:name/logs`) every 5s and re-renders the tail, auto-scrolled to
// the bottom — the React equivalent of the HTMX pages'
// `hx-trigger="load, every 5s"` log `<pre>`.
export function LogTerminal({ appName, lines = 300 }: { appName: string; lines?: number }) {
  const { data, isLoading } = useQuery({
    queryKey: ["app-logs", appName],
    queryFn: () => api.logs(appName, lines),
    refetchInterval: 5_000,
  });
  const preRef = useRef<HTMLPreElement>(null);

  useEffect(() => {
    const el = preRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [data]);

  return (
    <div className="terminal">
      <div className="terminal-bar">
        <span className="terminal-dot" />
        {appName} — live tail
      </div>
      <pre className="log" ref={preRef}>
        {isLoading ? "loading..." : data || "(no logs yet)"}
      </pre>
    </div>
  );
}
