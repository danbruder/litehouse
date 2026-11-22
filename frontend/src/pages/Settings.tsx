import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { ChevronDown, Copy } from "lucide-react";
import { api, type S3ConfigInput } from "../lib/api";
import { Button } from "../components/Button";
import { ConfirmButton } from "../components/ConfirmButton";
import { ErrorNote } from "../components/ErrorNote";
import { relativeTime } from "../lib/format";
import { Fact, Facts, Hint, Loading, Muted, Page, PageHeader, Section, copyText } from "../components/ui";

// S3 credentials and the GHCR read token used to be reachable only through
// `lh config ...`. Both endpoints redact their secrets server-side
// (`S3ConfigRedacted`, `redact_token`), so nothing secret is ever rendered
// back here; saving is always a full replacement.

function S3Section() {
  const qc = useQueryClient();
  const { data, error, isLoading, refetch } = useQuery({
    queryKey: ["s3-config"],
    queryFn: api.s3Config,
  });
  const [editing, setEditing] = useState(false);
  const [form, setForm] = useState<S3ConfigInput>({
    access_key_id: "",
    secret_access_key: "",
    bucket: "",
    region: "",
    endpoint: "",
    path_prefix: "",
  });

  const save = useMutation({
    mutationFn: () =>
      api.setS3Config({
        ...form,
        endpoint: form.endpoint?.trim() ? form.endpoint.trim() : null,
        path_prefix: form.path_prefix?.trim() ? form.path_prefix.trim() : null,
      }),
    onSuccess: () => {
      toast.success("S3 configuration saved");
      setEditing(false);
      setForm((f) => ({ ...f, secret_access_key: "" }));
      qc.invalidateQueries({ queryKey: ["s3-config"] });
    },
    onError: (err: Error) => toast.error("Couldn't save S3 configuration", { description: err.message }),
  });

  const remove = useMutation({
    mutationFn: api.deleteS3Config,
    onSuccess: () => {
      toast.success("S3 configuration removed. Backups stop until it's set again.");
      qc.invalidateQueries({ queryKey: ["s3-config"] });
    },
    onError: (err: Error) => toast.error("Couldn't remove S3 configuration", { description: err.message }),
  });

  const field = (
    label: string,
    key: keyof S3ConfigInput,
    opts: { type?: string; placeholder?: string; required?: boolean } = {},
  ) => (
    <label className="block">
      <span className="mb-1 block text-[11px] font-semibold uppercase tracking-[0.1em] text-ink-3">{label}</span>
      <input
        type={opts.type ?? "text"}
        required={opts.required ?? true}
        autoComplete="off"
        autoCapitalize="none"
        spellCheck={false}
        placeholder={opts.placeholder}
        value={(form[key] as string | null) ?? ""}
        onChange={(e) => setForm((f) => ({ ...f, [key]: e.target.value }))}
        className="w-full text-sm"
      />
    </label>
  );

  return (
    <Section
      title="S3 backups"
      action={
        <Button variant="ghost" onClick={() => setEditing((v) => !v)}>
          {editing ? "cancel" : data ? "replace" : "configure"}
        </Button>
      }
    >
      {error ? (
        <ErrorNote error={error} what="S3 configuration" onRetry={() => refetch()} />
      ) : isLoading ? (
        <Muted>loading…</Muted>
      ) : !data ? (
        <Muted>
          Not configured, so daily backups aren't running. <code>lh config s3 set</code> does the same from the
          CLI.
        </Muted>
      ) : (
        !editing && (
          <>
            <Facts>
              <Fact label="Bucket">
                {data.bucket} <span className="text-ink-3">({data.region})</span>
              </Fact>
              <Fact label="Access key">
                <span className="font-mono text-xs">{data.access_key_id}</span>
              </Fact>
              {data.endpoint && (
                <Fact label="Endpoint">
                  <span className="break-all">{data.endpoint}</span>
                </Fact>
              )}
              {data.path_prefix && <Fact label="Prefix">{data.path_prefix}</Fact>}
            </Facts>
            <div className="mt-3">
              <ConfirmButton
                variant="ghost"
                confirmLabel="remove S3 config"
                disabled={remove.isPending}
                onConfirm={() => remove.mutate()}
                className="-ml-2.5 !text-bad"
              >
                remove
              </ConfirmButton>
            </div>
          </>
        )
      )}

      {editing && (
        <form
          className="mt-1 grid grid-cols-1 gap-3 sm:grid-cols-2"
          onSubmit={(e) => {
            e.preventDefault();
            save.mutate();
          }}
        >
          {field("Access key id", "access_key_id")}
          {field("Secret access key", "secret_access_key", { type: "password" })}
          {field("Bucket", "bucket")}
          {field("Region", "region", { placeholder: "us-east-1" })}
          {field("Endpoint (optional)", "endpoint", {
            required: false,
            placeholder: "https://… for S3-compatible stores",
          })}
          {field("Path prefix (optional)", "path_prefix", { required: false })}
          <div className="sm:col-span-2">
            <Button variant="solid" size="md" type="submit" disabled={save.isPending} className="w-full sm:w-auto">
              {save.isPending ? "saving…" : "save configuration"}
            </Button>
            <Hint>Saving replaces the whole configuration. The secret key is write-only and never read back.</Hint>
          </div>
        </form>
      )}
    </Section>
  );
}

function GhcrSection() {
  const qc = useQueryClient();
  const { data, error, isLoading, refetch } = useQuery({
    queryKey: ["ghcr-config"],
    queryFn: api.ghcrConfig,
  });
  const [token, setToken] = useState("");

  const save = useMutation({
    mutationFn: () => api.setGhcrConfig(token.trim()),
    onSuccess: () => {
      toast.success("GHCR token saved");
      setToken("");
      qc.invalidateQueries({ queryKey: ["ghcr-config"] });
    },
    onError: (err: Error) => toast.error("Couldn't save GHCR token", { description: err.message }),
  });
  const remove = useMutation({
    mutationFn: api.deleteGhcrConfig,
    onSuccess: () => {
      toast.success("GHCR token removed");
      qc.invalidateQueries({ queryKey: ["ghcr-config"] });
    },
    onError: (err: Error) => toast.error("Couldn't remove GHCR token", { description: err.message }),
  });

  return (
    <Section
      title="GHCR read token"
      action={
        data?.configured ? (
          <ConfirmButton
            variant="ghost"
            confirmLabel="remove"
            disabled={remove.isPending}
            onConfirm={() => remove.mutate()}
          >
            remove
          </ConfirmButton>
        ) : undefined
      }
    >
      {error ? (
        <ErrorNote error={error} what="GHCR configuration" onRetry={() => refetch()} />
      ) : isLoading ? (
        <Muted>loading…</Muted>
      ) : data?.configured ? (
        <p className="m-0 mb-3 text-sm text-ink-2">
          Set: <span className="font-mono text-xs text-ink">{data.token}</span>
        </p>
      ) : (
        <Muted className="mb-3">Not set. Only needed to pull private images from ghcr.io.</Muted>
      )}

      <form
        onSubmit={(e) => {
          e.preventDefault();
          if (token.trim()) save.mutate();
        }}
      >
        <div className="flex flex-col gap-2 sm:flex-row sm:items-center">
          <input
            type="password"
            placeholder={data?.configured ? "New token" : "ghp_… / github_pat_…"}
            autoComplete="off"
            value={token}
            onChange={(e) => setToken(e.target.value)}
            className="sm:flex-1"
          />
          <Button variant="outline" size="md" type="submit" disabled={save.isPending || !token.trim()}>
            {data?.configured ? "replace" : "save"}
          </Button>
        </div>
      </form>
      <Hint>
        Needs only <code>read:packages</code>. Stored server-side, never shown again.
      </Hint>
    </Section>
  );
}

function ServerSection() {
  const { data, error, isLoading, refetch } = useQuery({
    queryKey: ["server-info"],
    queryFn: api.serverInfo,
    refetchInterval: 60_000,
  });

  return (
    <Section title="Server">
      {error ? (
        <ErrorNote error={error} what="server info" onRetry={() => refetch()} />
      ) : isLoading || !data ? (
        <Loading what="server info" />
      ) : (
        <Facts>
          <Fact label="Version">
            v{data.version}
            {data.local_dev && <span className="text-ink-3"> · local dev</span>}
          </Fact>
          <Fact label="Domain">
            {data.domain ? <span className="break-all">*.{data.domain}</span> : <span className="text-ink-3">not configured</span>}
          </Fact>
          {data.admin_host && (
            <Fact label="Admin host">
              <span className="break-all">{data.admin_host}</span>
            </Fact>
          )}
          <Fact label="Docker">
            {data.docker_version ?? <span className="text-ink-3">unknown</span>}
            {data.platform && <span className="text-ink-3"> · {data.platform}</span>}
          </Fact>
          <Fact label="Apps">
            {data.apps_running} running of {data.apps_total}
          </Fact>
        </Facts>
      )}
    </Section>
  );
}

// Remote MCP: the URL and ready-to-paste snippets for Claude (connector,
// Claude Code, .mcp.json), plus the OAuth grants already handed out. The
// admin token is never shown here: the cookie holding it is HttpOnly and the
// server only stores its hash, so token-bearing snippets use a placeholder.
// Prefer the OAuth flows, which need no token at all.
function Snippet({ label, hint, value }: { label: string; hint?: string; value: string }) {
  return (
    <div className="py-3 first:pt-0 last:pb-0">
      <div className="mb-1.5 text-[11px] font-semibold uppercase tracking-[0.1em] text-ink-3">{label}</div>
      <button
        type="button"
        onClick={() => copyText(value, label.toLowerCase())}
        title="Tap to copy"
        className="group flex w-full items-start gap-3 border border-solid border-rule bg-paper-2 p-3 text-left font-mono text-xs normal-case tracking-normal text-ink hover:border-ink hover:bg-paper-2 hover:text-ink"
      >
        <span className="min-w-0 flex-1 whitespace-pre-wrap break-all font-normal">{value}</span>
        <Copy size={14} className="mt-0.5 shrink-0 text-ink-3 group-hover:text-ink" />
      </button>
      {hint && <p className="m-0 mt-1.5 text-xs text-ink-3">{hint}</p>}
    </div>
  );
}

function McpSection() {
  const qc = useQueryClient();
  const url = `${window.location.origin}/mcp`;
  const { data, error, isLoading, refetch } = useQuery({
    queryKey: ["mcp-connections"],
    queryFn: api.mcpConnections,
  });
  const revoke = useMutation({
    mutationFn: api.revokeMcpConnection,
    onSuccess: () => {
      toast.success("Connection revoked");
      qc.invalidateQueries({ queryKey: ["mcp-connections"] });
    },
    onError: (err: Error) => toast.error("Couldn't revoke connection", { description: err.message }),
  });

  const mcpJson = JSON.stringify(
    {
      mcpServers: {
        litehouse: {
          type: "http",
          url,
          headers: { Authorization: "Bearer ${LITEHOUSE_TOKEN}" },
        },
      },
    },
    null,
    2,
  );

  return (
    <Section title="Connect Claude" flush>
      <div className="px-4 py-4 md:px-5">
        <p className="m-0 mb-4 text-sm text-ink-2">
          Let Claude deploy and manage apps here. The web, desktop and mobile apps sign in with OAuth: you approve
          once with your admin token, and can revoke access below.
        </p>
        <Snippet
          label="Connector URL"
          value={url}
          hint="claude.ai → Settings → Connectors → Add custom connector. It then works in the mobile app too."
        />
      </div>

      <details className="group border-0 border-t border-solid border-rule">
        <summary className="flex min-h-[48px] cursor-pointer list-none items-center justify-between px-4 text-xs font-semibold uppercase tracking-wider text-ink-2 hover:text-ink md:px-5 [&::-webkit-details-marker]:hidden">
          Claude Code and other setups
          <ChevronDown size={16} className="transition-transform group-open:rotate-180" />
        </summary>
        <div className="divide-y divide-rule px-4 pb-4 md:px-5">
          <Snippet
            label="Claude Code (OAuth)"
            value={`claude mcp add --transport http litehouse ${url}`}
            hint="Then run /mcp in Claude Code and choose litehouse to sign in."
          />
          <Snippet
            label="Claude Code (admin token)"
            value={`claude mcp add --transport http litehouse ${url} --header "Authorization: Bearer $LITEHOUSE_TOKEN"`}
            hint="Set LITEHOUSE_TOKEN to your admin token first. It's never shown on this page."
          />
          <Snippet
            label=".mcp.json (cloud sessions)"
            value={mcpJson}
            hint="Commit this to a repo and add LITEHOUSE_TOKEN as a secret in the cloud environment."
          />
          <Snippet label="Local stdio" value="claude mcp add litehouse -- lh mcp serve" hint="Uses the token from lh connect." />
        </div>
      </details>

      <div className="border-0 border-t border-solid border-rule">
        <div className="px-4 pb-1 pt-4 text-[11px] font-semibold uppercase tracking-[0.12em] text-ink-3 md:px-5">
          Connected clients
        </div>
        {error ? (
          <div className="px-4 pb-3">
            <ErrorNote error={error} what="MCP connections" onRetry={() => refetch()} />
          </div>
        ) : isLoading ? (
          <Muted className="px-4 pb-4 md:px-5">loading…</Muted>
        ) : !data || data.length === 0 ? (
          <Muted className="px-4 pb-4 md:px-5">None yet.</Muted>
        ) : (
          <ul className="m-0 list-none p-0">
            {data.map((c) => (
              <li
                key={c.id}
                className="flex min-h-[56px] items-center justify-between gap-3 border-0 border-b border-solid border-rule px-4 py-2 last:border-b-0 md:px-5"
              >
                <div className="min-w-0">
                  <div className="text-sm font-semibold text-ink">{c.client_name}</div>
                  <div className="text-xs text-ink-3">
                    {c.read_only ? "read-only" : "full access"} · last used{" "}
                    {c.last_used_at ? relativeTime(c.last_used_at) : "never"}
                  </div>
                </div>
                <ConfirmButton
                  variant="ghost"
                  confirmLabel="revoke"
                  disabled={revoke.isPending}
                  onConfirm={() => revoke.mutate(c.id)}
                >
                  revoke
                </ConfirmButton>
              </li>
            ))}
          </ul>
        )}
      </div>
    </Section>
  );
}

export function Settings() {
  return (
    <Page>
      <PageHeader title="Settings" />
      <ServerSection />
      <McpSection />
      <S3Section />
      <GhcrSection />
      <Section title="Maintenance">
        <Muted>
          Every running app is restarted once a night at 3am US Eastern. Opt one out with{" "}
          <code>lh env &lt;app&gt; LITEHOUSE_SKIP_NIGHTLY_RESTART true</code>.
        </Muted>
        <Muted className="mt-3">
          Disaster recovery stays a CLI operation on purpose: <code>lh restore --yes</code> rebuilds every app from
          GHCR + S3 and isn't something to trigger from a browser by accident.
        </Muted>
      </Section>
    </Page>
  );
}
