# litehouse admin dashboard (React)

The admin dashboard at `/` is a small React SPA that talks to litehouse's
existing JSON API (`src/api.rs` — the same one the `lh` CLI and MCP server
use). Everything else in the admin UI (`/apps/:name`, `/backups`, deploy
detail) is still server-rendered Askama + HTMX; only the dashboard has been
migrated so far. See `src/ui.rs`'s `spa_shell` handler for how this is
wired in.

## Why there's a build step here but "the server never builds anything"

litehouse's server never runs `docker build`, `npm install`, or any other
build at runtime — that principle doesn't change. This frontend is built
**once, by a person, on their machine** (or in CI, if that's ever added),
and its output (`dist/spa.js`, `dist/spa.css`) is committed straight into
`../src/ui/spa/`, exactly the way `htmx.min.js` and `styles.css` are
already vendored into `../src/ui/`. `cargo build` embeds those committed
files with `include_str!` — it never invokes `npm`. **If you change
anything under `frontend/src`, you must run `npm run build` and commit the
resulting changes in `../src/ui/spa/` yourself** — `cargo build` alone will
silently keep serving the old bundle otherwise.

## Dev loop

Two processes, both against the same local litehouse-server:

```bash
# Terminal 1 — the real backend, on :3030 (LITEHOUSE_LOCAL_DEV or a debug build)
cargo run -- serve

# Terminal 2 — Vite dev server with HMR, proxying /api, /login, /logout,
# and the two shared static assets to :3030 (see vite.config.ts)
cd frontend && npm run dev
```

Open the URL Vite prints (typically `http://localhost:5173`). Log in with
the admin token your local `lh serve` printed on first boot — the cookie
Vite's proxy forwards is the same one the real admin UI uses, so there's no
separate auth setup.

## Shipping a change

```bash
cd frontend
npm run build     # tsc -b && vite build, output -> ../src/ui/spa/
git add ../src/ui/spa
git commit
```

`npm run build` deliberately produces fixed, non-hashed filenames
(`spa.js`, `spa.css` — see `vite.config.ts`'s `rollupOptions.output`)
because `include_str!` paths are resolved at Rust compile time; a
content-hashed filename would mean editing `src/ui.rs` on every frontend
change just to update the include path.

## Stack

- Vite + React + TypeScript
- Tailwind, with `preflight` disabled and its color tokens mapped onto the
  existing `--color-*` CSS custom properties from `../src/ui/styles.css`
  (the "Ink & Lime" design system) — the SPA links that stylesheet before
  its own, and reuses its `.card`/`.badge`/`.btn-outline` classes directly
  rather than re-implementing them.
- `@tanstack/react-query` for data fetching/caching/polling (replaces the
  HTMX pages' `hx-trigger="every Ns"` polling with real invalidation +
  optimistic UI on start/stop/restart).
- `recharts` for the resource sparklines, `sonner` for toasts, `lucide-react`
  for icons, `class-variance-authority` + `tailwind-merge` for the couple of
  small component primitives in `src/components/`.

## Auth

The SPA never touches the admin token directly. It authenticates with the
same HttpOnly `litehouse_token` cookie the HTMX pages use (`credentials:
"include"` on every fetch — see `src/lib/api.ts`); `admin_auth_middleware`
(`src/auth.rs`) already accepts that cookie on `/api/*`, so no backend auth
changes were needed to reuse the JSON API from the browser.

## What's SPA-only vs. shared with the CLI

Most of `src/lib/api.ts` calls the same `/api/*` routes the CLI/MCP use.
Three endpoints exist only for this dashboard (see `src/api.rs`), kept
separate rather than folded into the CLI-facing ones so their JSON
contract can evolve independently:

- `GET /api/apps/summary` — apps with **live** container state (not the
  cached DB column), best-effort URL, and latest deploy info in one call.
- `POST /api/apps/:name/restart` — stop+start under the app's lock, same as
  the HTMX dashboard's restart button.
- `GET /api/metrics/server?hours=24` — raw CPU/mem/disk samples for the
  sparkline cards.

## Migrating the rest of the admin UI

Not done in this pass. The remaining Askama+HTMX pages (`/apps/:name`,
`/backups`, deploy detail) are linked from the dashboard as plain
`<a href>`s — full page navigations, not client-side routes — so clicking
into an app currently drops back into the old page style. Migrating a page
means: add whatever JSON endpoint it needs (most already exist), build the
React page under `src/pages/`, wire it into a client router (none exists
yet — a single-page dashboard didn't need one), and only then delete the
corresponding Askama template + `src/ui.rs` handler.
