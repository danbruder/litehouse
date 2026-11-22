import { useEffect, useRef } from "react";
import { useParams } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";
import { Copy } from "lucide-react";
import { LogTerminal } from "../components/LogTerminal";
import { ErrorNote } from "../components/ErrorNote";
import { Button } from "../components/Button";
import { api } from "../lib/api";
import { relativeTime, absoluteTime } from "../lib/format";
import { CopyButton, Fact, Facts, Hint, Loading, Muted, Page, PageHeader, Section, StatusText, copyText } from "../components/ui";

// There's no `GET /api/apps/:name/deploys/:id` endpoint — the app's deploy
// list already carries everything this page needs (and, at position 0,
// which deploy is the newest), so this just looks the id up client side.
export function DeployDetail() {
  const { name = "", deployId = "" } = useParams<{ name: string; deployId: string }>();
  const back = { to: `/apps/${encodeURIComponent(name)}?tab=deploys`, label: name };

  const { data: deploys, isLoading, error, refetch } = useQuery({
    queryKey: ["app-deploys", name, "all"],
    queryFn: () => api.deploys(name, 50),
    enabled: !!name,
    refetchInterval: (query) => {
      const deploy = query.state.data?.find((d) => d.id === deployId);
      return deploy?.status === "in_progress" ? 3_000 : false;
    },
  });
  const { data: summary } = useQuery({
    queryKey: ["app-summary", name],
    queryFn: () => api.appSummary(name),
    enabled: !!name,
  });

  const deploy = deploys?.find((d) => d.id === deployId);
  const isLatest = !!deploys && deploys.length > 0 && deploys[0].id === deployId;

  const logRef = useRef<HTMLPreElement>(null);
  useEffect(() => {
    const el = logRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [deploy?.log]);

  if (error)
    return (
      <Page>
        <PageHeader title="Deploy" back={back} />
        <ErrorNote error={error} what="this deploy" onRetry={() => refetch()} />
      </Page>
    );
  if (isLoading)
    return (
      <Page>
        <PageHeader title="Deploy" back={back} />
        <Loading what="deploy" />
      </Page>
    );
  if (!deploy)
    return (
      <Page>
        <PageHeader title="Deploy" back={back} />
        <Muted>That deploy isn't among this app's last 50.</Muted>
      </Page>
    );

  const tookSecs =
    deploy.status !== "in_progress" && deploy.updated_at
      ? Math.max(0, Math.round((Date.parse(deploy.updated_at) - Date.parse(deploy.created_at)) / 1000))
      : null;
  const repo = summary?.repo;

  return (
    <Page>
      <PageHeader
        back={back}
        title={
          <>
            Deploy <span className="font-mono text-[0.8em] font-medium text-ink-3">{deploy.id.slice(0, 8)}</span>
          </>
        }
        meta={
          <span className="flex flex-wrap items-center gap-x-3 gap-y-1">
            <StatusText state={deploy.status} />
            <span title={absoluteTime(deploy.created_at)}>
              {relativeTime(deploy.created_at)}
              {tookSecs != null && ` · took ${tookSecs}s`}
            </span>
          </span>
        }
      />

      {deploy.error && (
        <div className="mb-4 border border-solid border-bad px-4 py-3 md:mb-5">
          <div className="mb-1 text-[11px] font-semibold uppercase tracking-[0.12em] text-bad">Error</div>
          <p className="m-0 break-words text-sm text-ink">{deploy.error}</p>
        </div>
      )}

      <Section title="Details">
        <Facts>
          {deploy.git_sha && (
            <Fact label="Commit">
              <span className="flex items-center gap-1">
                {repo ? (
                  <a
                    href={`https://github.com/${repo}/commit/${deploy.git_sha}`}
                    target="_blank"
                    rel="noopener noreferrer"
                    className="break-all font-mono text-xs"
                  >
                    {deploy.git_sha}
                  </a>
                ) : (
                  <span className="break-all font-mono text-xs">{deploy.git_sha}</span>
                )}
                <CopyButton value={deploy.git_sha} label="commit" />
              </span>
            </Fact>
          )}
          <Fact label="Image">
            <span className="flex items-start gap-1">
              <span className="min-w-0 break-all font-mono text-xs text-ink-2">{deploy.image}</span>
              <CopyButton value={deploy.image} label="image" className="-mt-1.5 md:-mt-1" />
            </span>
          </Fact>
          <Fact label="Started">{absoluteTime(deploy.created_at)}</Fact>
        </Facts>
      </Section>

      <Section
        title={deploy.status === "in_progress" ? "Deploy log · in progress" : "Deploy log"}
        flush
        action={
          <Button variant="ghost" onClick={() => copyText(deploy.log || "", "deploy log")}>
            <Copy size={12} /> copy
          </Button>
        }
      >
        <div className="bg-paper-2">
          <pre className="log" ref={logRef}>
            {deploy.log || "(no log recorded for this deploy)"}
          </pre>
        </div>
      </Section>

      {isLatest && (
        <>
          <h2 className="m-0 mb-1 mt-8 font-display text-lg font-bold">App logs</h2>
          <Hint>Live output from the app's running container, not specific to this deploy.</Hint>
          <div className="mt-3">
            <LogTerminal appName={name} />
          </div>
        </>
      )}
    </Page>
  );
}
