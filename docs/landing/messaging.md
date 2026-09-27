# Landing page — messaging outline

Audience: developers who can read a Dockerfile and have run a VPS before.
Tone: a good README. State what it does, what it costs, what it won't do.
No adjectives we can't back with a command. No "blazing", "seamless",
"effortless", "magic". If a claim needs an asterisk, put the asterisk on
the page.

Positioning, one line (internal, not necessarily on the page):
**A self-hosted deploy platform for SQLite apps, on one box you own, with
backups and restore built in.**

---

## 1. Hero

**Headline:** `git push` deploys for SQLite apps, on your own server.

**Subhead:** Litehouse is a single binary that turns a $5 VPS into a
push-to-deploy host: HTTPS subdomains, daily S3 backups, and a one-command
restore onto a fresh machine. GitHub Actions does the builds; your server
just runs things.

**Primary CTA:** Install command, copyable:
```
curl -fsSL https://raw.githubusercontent.com/danbruder/litehouse/main/install.sh \
  | sudo sh -s -- --domain lh.example.com
```
**Secondary CTA:** GitHub repo link. (Maybe: "Read the source — it's Rust,
one binary.")

**Hero visual:** a terminal session, not an illustration. Real output:
```
$ lh create myapp --repo you/myapp
$ git push
$ lh deploys myapp --wait
✓ deployed a1b2c3d → https://myapp.lh.example.com
```
(Build the page with real captured output, not mocked-up output.)

---

## 2. The whole workflow, in four commands

Short section, one line of explanation per step. This is the pitch; most
readers decide here.

1. `install.sh --domain lh.example.com` — installs Docker bits, Caddy, the
   litehouse server container. Prints an admin token once.
2. `lh connect https://admin.lh.example.com --token …` — points your CLI at it.
3. `lh create myapp --repo you/myapp` — registers the app, commits a GitHub
   Actions workflow to your repo, sets the deploy secret. No CI yaml to write.
4. `git push` — Actions builds the image, pushes to GHCR, calls the deploy
   hook. Your app is at `https://myapp.lh.example.com`.

Footnote line: Your repo needs a Dockerfile. (Say it plainly — it's the main
prerequisite.)

---

## 3. What you get

Grid of short, factual cards. Each card = what + how, no fluff.

- **Push to deploy.** GitHub Actions builds; the server pulls from GHCR and
  swaps the container. The server never runs `docker build`, so a 1 GB box
  is fine.
- **HTTPS subdomains.** Caddy + Let's Encrypt. Every app gets
  `{app}.{your-domain}`. Custom domains via `lh domain add`.
- **Daily backups to S3.** Consistent SQLite snapshots (`VACUUM INTO`), plus
  the server's own state. 14-day retention. Any S3-compatible bucket.
- **Incremental file backups.** Write uploads to `$LITEHOUSE_BLOB_PATH`;
  each file is uploaded once and never re-sent.
- **Restore onto a fresh box.** `lh install` → `lh connect` →
  `lh restore --yes` rebuilds apps, images, env and data from S3 + GHCR.
- **Env vars, logs, start/stop/restart.** From the CLI or the admin UI.
- **A small admin UI.** Apps, deploy history, logs, metrics, backups. Same
  binary, one admin token.
- **An MCP server.** `lh mcp serve` exposes deploy, logs, env, domains,
  backups as tools for Claude Code or any MCP client.

---

## 4. Backups & recovery (the section that earns trust)

This is the main differentiator. Give it its own section, with details.

**Headline:** Assume the server dies.

**Body:** Nothing on the box is precious. Every night litehouse snapshots
each app's SQLite data and its own state database to your S3 bucket. If the
machine is gone, spin up a new one and run:
```
lh install --domain lh.example.com --s3-… --ghcr-token …
lh connect https://admin.lh.example.com --token …
lh restore --yes
```
Apps, deploy tokens, env vars, volumes and blobs come back. Images are
re-pulled from GHCR.

**Proof points (list):**
- A backup day is marked successful only if *every* app backed up with zero
  failures. `lh backup status --json` tells you the last good date.
- The full wipe → reinstall → restore cycle is an automated test
  (`e2e/dr-drill.sh`) run against a real droplet.
- Backups go to *your* bucket in plain `tar.gz`. You can restore without
  litehouse if you have to.

---

## 5. Built to be scripted (and driven by agents)

**Headline:** Every command works without a human.

- Non-interactive flags everywhere; `--json` on read commands.
- `lh deploys myapp --wait` blocks until the deploy finishes. Exit `0`
  success, `1` failed, `2` timeout. Use it in CI or let an agent check its
  own work.
- `lh mcp serve` for MCP clients. Example snippet: adding it to Claude Code
  config, then a short transcript: "deploy myapp and tell me if it's
  healthy" → tool calls → answer.

Keep this factual. The point isn't "AI deploys for you"; it's that an agent
can deploy *and confirm the result*.

---

## 6. What it costs

**Headline:** The price of a VPS.

- Litehouse: free, open source.
- Server: whatever your VPS costs. $5–6/mo handles a handful of small apps.
- Builds: GitHub Actions minutes (free tier covers most side projects).
- Images: GHCR (free for public; included with GitHub plans for private).
- Backups: S3-compatible storage — cents per month for typical SQLite apps.

One line: The 10th app costs the same as the first. No per-seat, per-app,
or per-request pricing, because there's no one to pay.

---

## 7. When not to use litehouse

Say this plainly on the page — it builds more trust than any feature card.

- **You need Postgres or MySQL.** Litehouse is built around SQLite on
  local disk. Use a managed PaaS.
- **You need more than one server** or horizontal scaling. It's
  single-host by design.
- **You want scale-to-zero.** Apps are always on (no cold starts, but no
  idle savings either).
- **You don't want to own a server at all.** You'll need a VPS, a domain
  with wildcard DNS, an S3 bucket, and a GitHub repo.
- **Your deploys can't tolerate a brief gap.** Deploys replace the container;
  Caddy retries during the swap, but it's not a blue/green system.
  (Revisit this line if zero-downtime ships.)
- **You have no Dockerfile.** Not supported yet (on the roadmap).

---

## 8. How it works (architecture)

One diagram and a few sentences. Devs want to know what's running.

Diagram: `git push` → GitHub Actions (build) → GHCR → deploy hook →
litehouse-server (on your VPS) → Docker → app containers ← Caddy ← users.
Side arrow: litehouse-server → S3 (nightly).

Short list of what's on the box:
- `litehouse-server` container — API, deploy hook, admin UI, scheduler
- `caddy` container — TLS + routing
- one container per app, SQLite data in a mounted volume
- `lh` CLI on the host for install/upgrade

Written in Rust. Server state is itself a SQLite database (and it's backed
up too).

---

## 9. Comparison (optional, keep it honest)

Small table, no checkmark-spam. Columns: litehouse / hosted PaaS
(Vercel, Render, Light Cloud-style) / general self-hosted PaaS (Coolify,
Dokku, Kamal).

Rows:
- Where it runs — your VPS / their cloud / your VPS
- Pricing — flat VPS cost / metered usage / flat VPS cost
- Database — SQLite on disk / managed Postgres (extra) / bring your own
- Backups + restore — built in, tested / varies, often DB-only / plugins
  or DIY
- Builds happen on — GitHub Actions / their builders / usually your server
- Scope — SQLite apps, one box / anything / anything

Line under the table: If you need Postgres or multiple servers, Coolify or
Kamal are good choices. Litehouse is narrower on purpose.

---

## 10. FAQ

- **Does it work with any language?** Anything with a Dockerfile that
  listens on a port.
- **Where does my SQLite file live?** In a Docker volume on the host, mounted
  at `/data`. Point your app there.
- **What happens during a deploy?** Pull new image → replace container →
  Caddy reloads. If the pull fails, the old container keeps running.
- **Can I deploy without GitHub?** `lh deploy myapp --image <ref>` deploys
  any image directly; the GitHub workflow is the default path, not the only
  one.
- **Private repos / images?** Yes; set a GHCR read token.
- **Why the nightly restart?** Every app gets a fresh container at 3am
  Eastern. Opt out per app with
  `lh env <app> LITEHOUSE_SKIP_NIGHTLY_RESTART true`.
- **ARM?** Yes — x86_64 and aarch64 servers (e.g. Hetzner CAX, Graviton,
  Ampere). Apps are built for whatever your server runs; `lh create` sets
  that up. Only true once a release containing multi-arch builds is cut.
- **How do I upgrade?** `lh upgrade`.
- **Is it production-ready?** Honest answer: it runs real apps for its
  author; it's young; backups mean you can recover from mistakes.

---

## 11. Footer CTA

Repeat the install command. Link: GitHub, `examples/hello` ("a minimal app
that deploys as-is"), docs/README.

Closing line: One box. `git push`. Backed up every night.

---

## Notes for the build

- Every command on the page must be real and copy-paste correct. Capture
  terminal output from an actual run (the `e2e/acceptance.sh` flow against
  lh.danbruder.com is a good source).
- Keep the page fast and static; no JS required to read it. Dark/light.
- Don't add testimonials, logos, or user counts until they're real.
- Before launch, check these claims against the code: custom domain TLS,
  metrics in the UI, deploy-gap behavior.
