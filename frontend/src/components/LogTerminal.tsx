import { useEffect, useMemo, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { toast } from "sonner";
import { ArrowDown, Copy, Download, Pause, Play, RotateCw, Search, WrapText } from "lucide-react";
import { api } from "../lib/api";
import { cn } from "../lib/cn";
import { ErrorNote } from "./ErrorNote";

const LINE_CHOICES = [100, 300, 1000, 5000];

// Polls the same one-shot log endpoint the CLI uses (`GET
// /api/apps/:name/logs`) and re-renders the tail, with the controls you
// reach for once a log is actually useful: pause (so the scrollback you're
// reading isn't yanked away), a filter, how many lines to pull, wrap, and
// copy/download. Icon-only tools on phones, labelled ones with a pointer.

const tool =
  "inline-flex h-10 min-w-[40px] items-center justify-center gap-1.5 border-0 bg-transparent px-2 text-[0.65rem] font-semibold uppercase tracking-wide text-ink-2 hover:bg-paper-3 hover:text-ink md:h-8";

export function LogTerminal({ appName, lines: initialLines = 300 }: { appName: string; lines?: number }) {
  const [paused, setPaused] = useState(false);
  const [lines, setLines] = useState(initialLines);
  const [filter, setFilter] = useState("");
  const [wrap, setWrap] = useState(true);
  const [pinned, setPinned] = useState(true);

  const { data, isLoading, error, refetch, isFetching } = useQuery({
    queryKey: ["app-logs", appName, lines],
    queryFn: () => api.logs(appName, lines),
    refetchInterval: paused ? false : 5_000,
  });

  const preRef = useRef<HTMLPreElement>(null);

  const shown = useMemo(() => {
    const text = data ?? "";
    if (!filter.trim()) return text;
    const needle = filter.toLowerCase();
    return text
      .split("\n")
      .filter((l) => l.toLowerCase().includes(needle))
      .join("\n");
  }, [data, filter]);

  // Only follow the tail while the view is pinned to the bottom.
  useEffect(() => {
    const el = preRef.current;
    if (el && pinned) el.scrollTop = el.scrollHeight;
  }, [shown, pinned]);

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
    <div className="mb-5 border border-solid border-ink bg-paper">
      <div className="flex flex-wrap items-center gap-x-1 gap-y-0 border-0 border-b border-solid border-rule px-2 py-1">
        <label className="order-first flex min-h-[44px] w-full items-center gap-2 border-0 border-b border-solid border-rule px-2 text-ink-3 md:order-none md:min-h-0 md:w-auto md:flex-1 md:border-b-0">
          <Search size={14} className="shrink-0" />
          <input
            type="text"
            placeholder="Filter lines"
            aria-label="Filter log lines"
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            className="!border-0 !bg-transparent !px-0 !py-1 text-sm"
          />
        </label>
        <button type="button" className={tool} onClick={() => setPaused((p) => !p)} title={paused ? "Resume" : "Pause"}>
          {paused ? <Play size={14} /> : <Pause size={14} />}
          <span className="hidden md:inline">{paused ? "resume" : "pause"}</span>
        </button>
        <select
          value={lines}
          onChange={(e) => setLines(Number(e.target.value))}
          aria-label="How many lines to tail"
          className="!border-0 !bg-transparent !py-1 text-xs"
        >
          {LINE_CHOICES.map((n) => (
            <option key={n} value={n}>
              {n} lines
            </option>
          ))}
        </select>
        <span className="flex-1 md:hidden" />
        <button type="button" className={tool} disabled={isFetching} onClick={() => refetch()} title="Refresh" aria-label="Refresh">
          <RotateCw size={14} className={isFetching ? "animate-spin" : undefined} />
        </button>
        <button
          type="button"
          className={cn(tool, wrap && "!text-ink")}
          onClick={() => setWrap((w) => !w)}
          title={wrap ? "Don't wrap lines" : "Wrap lines"}
          aria-label="Toggle line wrap"
          aria-pressed={wrap}
        >
          <WrapText size={14} />
        </button>
        <button type="button" className={tool} onClick={copy} title="Copy" aria-label="Copy logs">
          <Copy size={14} />
        </button>
        <button type="button" className={tool} onClick={download} title="Download" aria-label="Download logs">
          <Download size={14} />
        </button>
      </div>

      {error && (
        <div className="px-4">
          <ErrorNote error={error} what="logs" onRetry={() => refetch()} />
        </div>
      )}

      <div className="relative bg-paper-2">
        <div className="flex items-center gap-2 border-0 border-b border-solid border-rule px-4 py-1.5 text-[10px] font-semibold uppercase tracking-wider text-ink-3">
          <span className={cn("dot !h-1.5 !w-1.5", paused ? "text-ink-3" : "dot-pulse text-good")} />
          {paused ? "paused" : "live"}
          {filtered && ` · ${hiddenCount} lines hidden`}
        </div>
        <pre
          className="log"
          ref={preRef}
          style={wrap ? { whiteSpace: "pre-wrap", wordBreak: "break-word" } : { whiteSpace: "pre", wordBreak: "normal" }}
          onScroll={(e) => {
            const el = e.currentTarget;
            const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
            if (atBottom !== pinned) setPinned(atBottom);
          }}
        >
          {isLoading ? "loading…" : shown || (filtered ? "(no lines match the filter)" : "(no logs yet)")}
        </pre>
        {!pinned && (
          <button
            type="button"
            onClick={() => setPinned(true)}
            className="absolute bottom-3 right-3 inline-flex h-9 items-center gap-1.5 border border-solid border-ink bg-paper px-3 text-[0.65rem] font-semibold uppercase tracking-wide text-ink hover:bg-lime hover:text-on-lime"
          >
            <ArrowDown size={13} /> latest
          </button>
        )}
      </div>
    </div>
  );
}
