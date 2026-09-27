# Deploying with litehouse — guide for AI agents

litehouse runs Docker apps on one server the user owns. GitHub Actions builds
each app's image and pushes it to GHCR; the server pulls it, swaps the
container, and serves it at `https://<app>.<domain>` with HTTPS. Each app's
`/data` directory is backed up to S3 every night.

You drive it with the `lh` CLI (or its MCP server, `lh mcp serve`). Every
command is non-interactive. Read commands take `--json`, and exit codes mean
something: 0 is success, anything else is a failure you should read.

This guide is also built into the CLI: `lh agent-guide` prints the version
that matches the installed `lh`.

## Rules

- **Never ask for, print, or commit the admin token.** The user connects the
  CLI themselves (`lh connect …`), or the environment provides
  `LITEHOUSE_URL` and `LITEHOUSE_TOKEN`. If `lh doctor` reports no server
  connection, stop and ask the user to run `lh connect`. Don't ask them to
  paste the token into the chat.
- **Don't write deploy tokens into files.** `lh create` stores the app's
  deploy token as a GitHub Actions secret on its own.
- **Ask the user before you run** `lh delete`, `lh restore`, or
  `lh create --rotate-token` on an app that already exists. They're
  destructive or they replace secrets.
- **After every `git push`, confirm with `--sha`.** Run
  `lh deploys <app> --wait --sha HEAD`. Without `--sha`, you wait on the
  newest deploy, which right after a push is still the *previous* one, so
  you'd see its old result.

## 1. Preflight

```sh
lh --version                # not found? install it (below), or ask the user to
lh doctor --app <app>       # checks every prerequisite; fix each FAIL, then re-run
```

Install the CLI with no root access. It lands in `~/.local/bin`:

```sh
curl -fsSL https://raw.githubusercontent.com/danbruder/litehouse/main/install-cli.sh | sh
```

`lh doctor` checks four things:

- **The server connection and admin token.**
- **A GitHub token with the `repo` and `workflow` scopes.** It's taken from
  `$GITHUB_TOKEN`, `gh auth token`, or `lh github login`. `workflow` is
  required because `lh create` commits a workflow file. If it's missing, the
  user runs `gh auth refresh -h github.com -s workflow`. That command is
  interactive, so ask them.
- **An `origin` remote that points at github.com.**
- **A `Dockerfile` at the repo root, and the port it `EXPOSE`s.**

## 2. What the app must look like

| Requirement | Details |
|---|---|
| `Dockerfile` at the repo root | The workflow runs `docker build .` on GitHub Actions for the server's platform. `lh doctor` shows it, e.g. `linux/amd64`. |
| Listen on `0.0.0.0:<port>` and `EXPOSE <port>` | HTTPS traffic goes to the lowest `EXPOSE`d TCP port, or 3000 if there's none. Binding to `localhost` makes the app unreachable. |
| Persistent data only under `/data` | `/data` is a volume that survives deploys and restarts. Everything else in the container is thrown away on each deploy and on the nightly restart. |
| SQLite in `/data` | Use `/data/app.db` (an empty one is created for you). Every `*.db`, `*.sqlite` and `*.sqlite3` under `/data` is snapshotted nightly with `VACUUM INTO`. Other files in `/data` go into the same nightly tarball. |
| Uploads and large files in `$LITEHOUSE_BLOB_PATH` | Set to `/data/blobs`. Each file there is uploaded to S3 once, never re-uploaded. |
| Fit in 256 MB of RAM | That's the default limit. Raise it with `lh env <app> LITEHOUSE_MEMORY_LIMIT_MB 512`. |
| Configuration through env vars | `lh env <app> KEY VALUE`, then `lh restart <app>`. Env changes apply to the next container start. |
| App name is a DNS label | 1–63 lowercase letters, digits and hyphens. It becomes the subdomain. |

The volume is chowned to the image's `USER`, so a non-root user can write
`/data`.

A minimal Dockerfile, as a shape to adapt:

```dockerfile
FROM node:22-slim
WORKDIR /app
COPY package*.json ./
RUN npm ci --omit=dev
COPY . .
ENV PORT=8080 DATABASE_PATH=/data/app.db
EXPOSE 8080
CMD ["node", "server.js"]
```

An optional health check lets Caddy route around a container that's
starting up: `lh health-check set <app> /healthz`.

## 3. First deploy

```sh
# 1. The Dockerfile must already be on the default branch (main or master),
#    because creating the app triggers a build straight away.
git add -A && git commit -m "Add Dockerfile" && git push

# 2. Register the app. This commits .github/workflows/litehouse-deploy.yml
#    to the repo on GitHub, sets the LITEHOUSE_DEPLOY_TOKEN secret, and
#    that commit triggers the first build.
lh create <app> --repo <owner>/<repo> --json

# 3. Get the workflow commit lh create just made, then wait for its deploy.
git pull --ff-only
lh deploys <app> --wait --sha HEAD --timeout 900
```

The workflow only runs on pushes to `main` or `master`.

## 4. Every deploy after that

```sh
git push
lh deploys <app> --wait --sha HEAD --timeout 900
```

| Exit | Meaning | What to do |
|---|---|---|
| 0 | The deploy for that commit succeeded and the app is live. | Check it (step 5). |
| 1, "Deploy … failed" | The image was pulled but the container didn't start or run. | `lh logs <app> -l 200`, fix, push again. The previous version keeps serving if the pull itself failed. |
| 1, "GitHub Actions run … failed" | The image build (or the call to the deploy hook) failed on GitHub. | `gh run view <id> --log-failed`. The run id is in the message. |
| 2 | Timed out: no deploy for that commit showed up, or it's still running. | `gh run list --workflow litehouse-deploy.yml`. Check that you pushed to `main`/`master` and that the workflow file exists. |

To ship an image you built yourself, skipping GitHub Actions:
`lh deploy <app> --image <ref> --sha <sha>`.

## 5. Verify

```sh
lh status <app>
curl -fsS https://<app>.<domain>/     # the first request may take a few seconds while the certificate is issued
lh logs <app> -l 100
```

## Other operations

| Task | Command |
|---|---|
| Set or remove an env var | `lh env <app> KEY VALUE` / `lh env <app> KEY "" --delete`, then `lh restart <app>` |
| Logs | `lh logs <app> -l 200` (`-f` follows; don't use it in non-interactive runs) |
| Deploy history | `lh deploys <app> --json` |
| Custom domain | `lh domain add <app> example.com` (point its DNS at the server first) |
| Backups | `lh backup status --json`; `lh backup run` backs up now |
| Stop, start, restart | `lh stop <app>`, `lh start <app>`, `lh restart <app>` |
| Every app | `lh status` |

## MCP

Register the MCP server with Claude Code (the same works for any MCP client):

```sh
claude mcp add litehouse -- lh mcp serve
```

It uses the same connection as the CLI: `lh connect` or `LITEHOUSE_URL`
and `LITEHOUSE_TOKEN`. The tools match the commands above: `create_app`,
`list_deploys`, `deploy`, `logs`, `env_set`, `app_status`, `list_apps`,
`start_app`, `stop_app`, `delete_app`, the `*_domain` tools, `backup_status`,
`run_backup`, and `agent_guide`, which returns this guide. After a push,
call `list_deploys` with `wait: true` and `sha: "HEAD"`. It returns an
error result unless that commit's deploy succeeded.
