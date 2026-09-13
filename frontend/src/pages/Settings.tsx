import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { api, type S3ConfigInput } from "../lib/api";
import { Button } from "../components/Button";
import { ConfirmButton } from "../components/ConfirmButton";
import { ErrorNote } from "../components/ErrorNote";

// Server-wide settings had no UI at all before this page: S3 credentials and
// the GHCR read token were reachable only through `lh config ...`, which
// means a fresh install couldn't be finished — or even audited — from the
// browser. Both endpoints already redact their secrets server-side
// (`S3ConfigRedacted`, `redact_token`), so nothing secret is ever rendered
// back here; saving is always a full replacement.
function S3Card() {
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
      toast.success("S3 configuration removed — backups will stop until it's set again");
      qc.invalidateQueries({ queryKey: ["s3-config"] });
    },
    onError: (err: Error) => toast.error("Couldn't remove S3 configuration", { description: err.message }),
  });

  const field = (
    label: string,
    key: keyof S3ConfigInput,
    opts: { type?: string; placeholder?: string; required?: boolean } = {},
  ) => (
    <label className="block text-xs text-ink-2">
      {label}
      <input
        type={opts.type ?? "text"}
        required={opts.required ?? true}
        autoComplete="off"
        placeholder={opts.placeholder}
        value={(form[key] as string | null) ?? ""}
        onChange={(e) => setForm((f) => ({ ...f, [key]: e.target.value }))}
        className="mt-1 w-full text-xs"
      />
    </label>
  );

  return (
    <div className="card">
      <div className="panel-header">
        <span className="panel-label">s3 backups</span>
        <Button variant="ghost" onClick={() => setEditing((v) => !v)}>
          {editing ? "cancel" : data ? "replace" : "configure"}
        </Button>
      </div>

      {error ? (
        <ErrorNote error={error} what="S3 configuration" onRetry={() => refetch()} />
      ) : isLoading ? (
        <p className="muted">loading…</p>
      ) : !data ? (
        <p className="muted">
          not configured — daily backups are not running. <code>lh config s3 set</code> does the same
          thing from the CLI.
        </p>
      ) : (
        <>
          <p>
            <strong>Bucket:</strong> <code>{data.bucket}</code> <span className="muted">({data.region})</span>
          </p>
          <p>
            <strong>Access key:</strong> <code>{data.access_key_id}</code>
          </p>
          {data.endpoint && (
            <p>
              <strong>Endpoint:</strong> <code>{data.endpoint}</code>
            </p>
          )}
          {data.path_prefix && (
            <p>
              <strong>Prefix:</strong> <code>{data.path_prefix}</code>
            </p>
          )}
          <p>
            <ConfirmButton
              variant="ghost"
              confirmLabel="remove S3 config"
              disabled={remove.isPending}
              onConfirm={() => remove.mutate()}
            >
              remove
            </ConfirmButton>
          </p>
        </>
      )}

      {editing && (
        <form
          className="mt-3 grid grid-cols-1 gap-3 border-t border-rule pt-3 sm:grid-cols-2"
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
            placeholder: "https://... for S3-compatible stores",
          })}
          {field("Path prefix (optional)", "path_prefix", { required: false })}
          <div className="sm:col-span-2">
            <Button variant="solid" type="submit" disabled={save.isPending}>
              {save.isPending ? "saving…" : "save configuration"}
            </Button>
            <span className="hint">
              Saving replaces the whole configuration — the secret key is write-only and never read back.
            </span>
          </div>
        </form>
      )}
    </div>
  );
}

function GhcrCard() {
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
    <div className="card">
      <span className="panel-label">ghcr read token</span>
      {error ? (
        <ErrorNote error={error} what="GHCR configuration" onRetry={() => refetch()} />
      ) : isLoading ? (
        <p className="muted">loading…</p>
      ) : data?.configured ? (
        <p className="flex flex-wrap items-center gap-3">
          <span>
            <strong>Token:</strong> <code>{data.token}</code>
          </span>
          <ConfirmButton
            variant="ghost"
            confirmLabel="remove token"
            disabled={remove.isPending}
            onConfirm={() => remove.mutate()}
          >
            remove
          </ConfirmButton>
        </p>
      ) : (
        <p className="muted">
          not set — only needed to pull <em>private</em> images from ghcr.io.
        </p>
      )}

      <form
        className="env-form"
        onSubmit={(e) => {
          e.preventDefault();
          if (token.trim()) save.mutate();
        }}
      >
        <input
          type="password"
          placeholder={data?.configured ? "replace token" : "ghp_… / github_pat_…"}
          autoComplete="off"
          value={token}
          onChange={(e) => setToken(e.target.value)}
        />
        <button type="submit" disabled={save.isPending || !token.trim()}>
          save
        </button>
      </form>
      <p className="hint">Needs only <code>read:packages</code>. Stored server-side, never shown again.</p>
    </div>
  );
}

function ServerCard() {
  const { data, error, isLoading, refetch } = useQuery({
    queryKey: ["server-info"],
    queryFn: api.serverInfo,
    refetchInterval: 60_000,
  });

  return (
    <div className="card">
      <span className="panel-label">server</span>
      {error ? (
        <ErrorNote error={error} what="server info" onRetry={() => refetch()} />
      ) : isLoading || !data ? (
        <p className="muted">loading…</p>
      ) : (
        <>
          <p>
            <strong>litehouse:</strong> <code>v{data.version}</code>
            {data.local_dev && <span className="muted"> · local dev</span>}
          </p>
          <p>
            <strong>Domain:</strong>{" "}
            {data.domain ? <code>*.{data.domain}</code> : <span className="muted">not configured</span>}
          </p>
          {data.admin_host && (
            <p>
              <strong>Admin host:</strong> <code>{data.admin_host}</code>
            </p>
          )}
          <p>
            <strong>Docker:</strong>{" "}
            {data.docker_version ? <code>{data.docker_version}</code> : <span className="muted">unknown</span>}
          </p>
          <p>
            <strong>Apps:</strong> {data.apps_running} running of {data.apps_total}
          </p>
        </>
      )}
    </div>
  );
}

export function Settings() {
  return (
    <>
      <h2>Settings</h2>
      <ServerCard />
      <S3Card />
      <GhcrCard />
      <div className="card">
        <span className="panel-label">maintenance</span>
        <p className="muted">
          Every running app is restarted once a night at 3am US Eastern. Opt one out with{" "}
          <code>lh env set &lt;app&gt; LITEHOUSE_SKIP_NIGHTLY_RESTART true</code>.
        </p>
        <p className="muted">
          Disaster recovery stays a CLI operation on purpose: <code>lh restore --yes</code> rebuilds
          every app from GHCR + S3 and is not something to trigger from a browser by accident.
        </p>
      </div>
    </>
  );
}
