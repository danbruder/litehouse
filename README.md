# Litehouse

A self-hosted, single-server platform for SQLite apps. `lh create` once, `git push` to deploy forever — similar to Vercel, but it's your own box.

The server never builds anything. GitHub Actions builds each app's image and pushes it to GHCR; the server just pulls, runs, routes, and backs up.

## How it works

- `lh create myapp --repo owner/myapp` registers the app, commits a ready-made GitHub Actions workflow to the repo, and sets a per-app deploy secret — no manual CI setup.
- `git push` builds the image on GitHub, pushes it to `ghcr.io/owner/myapp`, and calls the server's deploy hook, which pulls the image, replaces the running container, and updates the Caddy reverse proxy (automatic HTTPS, subdomain routing).
- A built-in daily job snapshots every app's SQLite data (and the server's own state) to S3; `lh restore --yes` on a freshly installed server rebuilds everything — apps, images, data — from GHCR + S3.
- Apps get a `LITEHOUSE_BLOB_PATH` env var (currently `/data/blobs`) for storing binary blobs (photos, attachments, etc.) that don't belong in the daily SQLite/tarball snapshot. Files written there back up incrementally — uploaded to S3 once, never re-uploaded on later days if unchanged — instead of riding along in the full daily tarball. See `docs/superpowers/specs/2026-07-14-blob-backup-design.md`.

## Quickstart

Install on a fresh Linux host with wildcard DNS already pointed at it (`*.lh.example.com` → server IP):

```bash
curl -fsSL https://raw.githubusercontent.com/danbruder/litehouse/main/install.sh | sudo sh -s -- --domain lh.example.com
```

This prints an admin token once — save it. Point the CLI at the server:

```bash
lh connect https://admin.lh.example.com --token <TOKEN>
```

Create and deploy an app from a GitHub repo:

```bash
lh create myapp --repo you/myapp
git push   # builds on GitHub, deploys automatically
lh deploys myapp --wait   # blocks until the deploy succeeds or fails
```

Your app is live at `https://myapp.lh.example.com`. See `examples/hello` for a minimal repo that works out of the box.

### Optional: backups and private images at install time

```bash
sudo lh install --domain lh.example.com \
  --s3-access-key ... --s3-secret-key ... --s3-bucket ... --s3-region us-east-1 \
  --ghcr-token ghp_...   # only needed to pull private ghcr.io images
```

Credentials can also be set (or changed) later without reinstalling:

```bash
lh config s3 set --access-key-id ... --secret-access-key ... --bucket ... --region us-east-1
lh config ghcr set --token ghp_...
```

## Backups and disaster recovery

Once S3 is configured, the server backs up every app's SQLite data (via `VACUUM INTO` snapshots) plus its own state database once a day, retaining the last 14 days. Run `lh backup run` for an on-demand backup or `lh backup status --json` to check the last successful date. To recover onto a brand-new node: `lh install --domain ... --s3-* --ghcr-token ...`, then `lh connect`, then `lh restore --yes` — it pulls the latest state snapshot from S3, re-pulls every app's image from GHCR, restores each app's data volume from its own S3 backup, and starts the containers.

## Commands

Run `lh --help` or `lh <command> --help` for full details; flags below are the notable ones.

| Command | What it does |
|---|---|
| `lh install --domain <domain>` | Install litehouse on this server (run as root); accepts `--s3-*` and `--ghcr-token` to configure backups/private images up front |
| `lh upgrade [--version <v>]` | Upgrade the litehouse binary and container image |
| `lh connect <url> --token <token>` | Point this CLI at a server |
| `lh create <app> [--repo owner/name] [--rotate-token] [--json]` | Register an app, commit its deploy workflow, set the deploy secret |
| `lh delete <app>` | Delete an app (stops the container, removes DB records) |
| `lh deploy <app> --image <ref> [--sha <sha>]` | Deploy an image directly (the local escape hatch — same path the deploy hook uses) |
| `lh deploys <app> [--limit N] [--json] [--wait] [--timeout secs]` | List deploy history, or wait for the in-flight deploy to finish |
| `lh start` / `lh stop` / `lh restart <app>` | Container lifecycle |
| `lh status [app]` | Show one app's or all apps' status |
| `lh logs <app> [-l N] [-f]` | View (optionally follow) container logs |
| `lh env <app> <key> <value> [--delete]` | Set or delete an environment variable |
| `lh check-dns` | Check wildcard DNS for the configured domain |
| `lh github login` | Device-flow OAuth, used by `lh create` to commit workflows and set secrets |
| `lh config s3 set/get/delete` | Configure S3 backup credentials |
| `lh config ghcr set/get/delete` | Configure the GHCR read token used to pull private images |
| `lh backup run` | Run a full backup now and print the report |
| `lh backup status [--json]` | Show the last backup date and report |
| `lh restore [--yes]` | Restore all apps from the newest S3 backup (disaster recovery) |
| `lh serve` | Start the server (admin API, deploy hook, UI, backup scheduler) — runs inside the `litehouse-server` container |

### Checking server status

```bash
docker ps | grep litehouse-server   # is the server container running
docker logs litehouse-server -f     # server logs
docker ps | grep caddy-container    # reverse proxy
docker restart litehouse-server     # restart
```

**Note:** litehouse runs as a Docker container, not a systemd service — don't reach for `systemctl`.

## Development

```bash
# Build
cargo build

# Run the server locally
cargo run -- serve

# Run tests
cargo test

# Some Docker-integration tests are ignored by default (real containers, MinIO
# round-trips); they need a pinned Docker API version:
DOCKER_API_VERSION=1.42 cargo test test_backup_roundtrip_minio -- --ignored --nocapture

# Build for production (Linux musl target)
TARGET_CC=x86_64-linux-musl-gcc cargo build --release --target x86_64-unknown-linux-musl
```

### Releasing litehouse itself

`./release.sh <version>` bumps `Cargo.toml`, commits, tags `v<version>`, and pushes. `.github/workflows/release.yml` then builds the `lh` binary, attaches it to a GitHub Release, and pushes `ghcr.io/danbruder/litehouse:<version>`/`:latest` to GHCR. Its final `deploy` job SSHes into the production server and runs `lh upgrade`, which pulls the new image, restarts the `litehouse-server` container, and updates the host `lh` binary — so pushing a tag is enough to ship a new litehouse release end-to-end.

That job needs these repo secrets (Settings → Secrets and variables → Actions):

| Secret | Value |
| --- | --- |
| `LITEHOUSE_SERVER_HOST` | Server hostname/IP (e.g. `lh.danbruder.com`) |
| `LITEHOUSE_SERVER_USER` | SSH user (e.g. `root`) |
| `LITEHOUSE_SERVER_SSH_KEY` | Private key for a keypair authorized on the server, dedicated to CI |

`dev-deploy.sh` remains for local iteration: it builds the binary on your machine and runs `lh upgrade --from-path` over SSH, without waiting on a tagged release.

### SQLx offline mode

Queries are checked at compile time against `.sqlx/` (checked into the repo). After any migration or query change, regenerate it against a live dev database and commit the result:

```bash
DATABASE_URL="sqlite:$PWD/.dev/litehouse.db" cargo sqlx database create
DATABASE_URL="sqlite:$PWD/.dev/litehouse.db" cargo sqlx migrate run
DATABASE_URL="sqlite:$PWD/.dev/litehouse.db" cargo sqlx prepare
```

### End-to-end scripts

`e2e/acceptance.sh` and `e2e/dr-drill.sh` drive the full flow against a real droplet (they `ssh`/`scp`, wipe the node, install, and push a real app). Both need `SERVER_IP`, `DOMAIN`, and (for acceptance) `HELLO_REPO` env vars; see the scripts for the full list of inputs (`S3_ARGS`, `GHCR_ARGS`, `PREBUILT_LH`). `examples/hello` is the reference app both scripts deploy.

## License

MIT
