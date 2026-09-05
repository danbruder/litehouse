import { useEffect, useRef } from "react";
import { Link, useParams } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";
import { DeployBadge } from "../components/StatusBadge";
import { LogTerminal } from "../components/LogTerminal";
import { api } from "../lib/api";
import { relativeTime } from "../lib/format";

// There's no `GET /api/apps/:name/deploys/:id` endpoint — the app's deploy
// list already carries everything this page needs (and, at position 0,
// which deploy is the current one), so this just looks the id up client
// side rather than adding a single-deploy endpoint for one page.
export function DeployDetail() {
  const { name = "", deployId = "" } = useParams<{ name: string; deployId: string }>();

  const { data: deploys, isLoading } = useQuery({
    queryKey: ["app-deploys", name, "all"],
    queryFn: () => api.deploys(name, 50),
    enabled: !!name,
    refetchInterval: (query) => {
      const list = query.state.data;
      const deploy = list?.find((d) => d.id === deployId);
      return deploy?.status === "in_progress" ? 3_000 : false;
    },
  });

  const deploy = deploys?.find((d) => d.id === deployId);
  const isLatest = !!deploys && deploys.length > 0 && deploys[0].id === deployId;

  const logRef = useRef<HTMLPreElement>(null);
  useEffect(() => {
    const el = logRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [deploy?.log]);

  return (
    <>
      <p>
        <Link to={`/apps/${encodeURIComponent(name)}`}>&larr; {name}</Link>
      </p>

      {isLoading ? (
        <p className="muted">loading…</p>
      ) : !deploy ? (
        <p className="muted">deploy not found</p>
      ) : (
        <>
          <h2>
            deploy <span className="muted">{deploy.id.slice(0, 8)}</span> <DeployBadge status={deploy.status} />
          </h2>

          <div className="card">
            <span className="panel-label">deploy</span>
            <p>
              <strong>Image:</strong> <code>{deploy.image}</code>
            </p>
            {deploy.git_sha && (
              <p>
                <strong>Commit:</strong> <code>{deploy.git_sha}</code>
              </p>
            )}
            <p>
              <strong>Started:</strong> {relativeTime(deploy.created_at)}
            </p>
            {deploy.error && (
              <>
                <p>
                  <strong>Error:</strong>
                </p>
                <div className="deploy-error-body">{deploy.error}</div>
              </>
            )}
          </div>

          <h3>Deploy log</h3>
          <div className="terminal">
            <div className="terminal-bar">
              <span className="terminal-dot" />
              deploy {deploy.id.slice(0, 8)}
              {deploy.status === "in_progress" ? " — in progress" : ""}
            </div>
            <pre className="log" ref={logRef}>
              {deploy.log || "(no log recorded for this deploy)"}
            </pre>
          </div>

          {isLatest && (
            <>
              <h3>App logs</h3>
              <p className="hint">Live output from the app's running container, not specific to this deploy.</p>
              <LogTerminal appName={name} />
            </>
          )}
        </>
      )}
    </>
  );
}
