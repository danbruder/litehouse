import { useEffect, useMemo, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { toast } from "sonner";
import { Pause, Play, Copy, Download, RotateCw } from "lucide-react";
import { api } from "../lib/api";
import { Button } from "./Button";
import { ErrorNote } from "./ErrorNote";

const LINE_CHOICES = [100, 300, 1000, 5000];

// Polls the same one-shot log endpoint the CLI uses (`GET
// /api/apps/:name/logs`) and re-renders the tail. Beyond the original
// auto-scrolling <pre>, this adds the controls you reach for the moment a
// log is actually useful: pause (so a scrollback you're reading doesn't get
// yanked out from under you), a filter, how many lines to pull, and
// copy/download for pasting into an issue.
export function LogTerminal({ appName, lines: initialLines = 300 }: { appName: string; lines?: number }) {
  const [paused, setPaused] = useState(false);
  const [lines, setLines] = useState(initialLines);
  const [filter, setFilter] = useState("");
  const [wrap, setWrap] = useState(true);

  const { data, isLoading, error, refetch, isFetching } = useQuery({
    queryKey: ["app-logs", appName, lines],
    queryFn: () => api.logs(appName, lines),
    refetchInterval: paused ? false : 5_000,
  });

  const preRef = useRef<HTMLPreElement>(null);
  // Only auto-scroll when the view is already pinned to the bottom —
  // scrolling up to read something no longer fights the 5s refresh.
  const pinnedRef = useRef(true);

  const shown = useMemo(() => {
    const text = data ?? "";
    if (!filter.trim()) return text;
    const needle = filter.toLowerCase();
    return text
      .split("\n")
      .filter((l) => l.toLowerCase().includes(needle))
      .join("\n");
  }, [data, filter]);

  useEffect(() => {
    const el = preRef.current;
    if (el && pinnedRef.current) el.scrollTop = el.scrollHeight;
  }, [shown]);

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(shown);
      toast.success("Logs copied");
    } catch {
      toast.error("Couldn't copy to clipboard");
    }
  };

  const download = () => {
    const blob = new Blob([shown], { type: "text/plain" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `${appName}-logs.txt`;
    a.click();
    URL.revokeObjectURL(url);
  };

  const filtered = filter.trim().length > 0;
  const hiddenCount = filtered ? (data ?? "").split("\n").length - shown.split("\n").length : 0;

  return (
    <>
      <div className="mb-2 flex flex-wrap items-center gap-2">
        <input
          type="text"
          placeholder="filter lines…"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          className="min-w-[10rem] flex-1 text-xs"
        />
        <select
          value={lines}
          onChange={(e) => setLines(Number(e.target.value))}
          title="how many lines to tail"
          className="border border-rule bg-paper px-2 py-1 text-xs text-ink"
        >
          {LINE_CHOICES.map((n) => (
            <option key={n} value={n}>
              {n} lines
            </option>
          ))}
        </select>
        <Button variant="outline" onClick={() => setPaused((p) => !p)}>
          {paused ? (
            <>
              <Play size={12} /> resume
            </>
          ) : (
            <>
              <Pause size={12} /> pause
            </>
          )}
        </Button>
        <Button variant="ghost" disabled={isFetching} onClick={() => refetch()}>
          <RotateCw size={12} /> refresh
        </Button>
        <Button variant="ghost" onClick={() => setWrap((w) => !w)}>
          {wrap ? "no wrap" : "wrap"}
        </Button>
        <Button variant="ghost" onClick={copy}>
          <Copy size={12} /> copy
        </Button>
        <Button variant="ghost" onClick={download}>
          <Download size={12} /> download
        </Button>
      </div>

      {error && <ErrorNote error={error} what="logs" onRetry={() => refetch()} />}

      <div className="terminal">
        <div className="terminal-bar">
          <span className="terminal-dot" />
          {appName} — {paused ? "paused" : "live tail"}
          {filtered && ` · filtered (${hiddenCount} lines hidden)`}
        </div>
        <pre
          className="log"
          ref={preRef}
          style={wrap ? { whiteSpace: "pre-wrap", wordBreak: "break-word" } : undefined}
          onScroll={(e) => {
            const el = e.currentTarget;
            pinnedRef.current = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
          }}
        >
          {isLoading ? "loading..." : shown || (filtered ? "(no lines match the filter)" : "(no logs yet)")}
        </pre>
      </div>
    </>
  );
}
