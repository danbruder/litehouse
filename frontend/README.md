# litehouse admin dashboard (React)

The whole admin UI — the dashboard at `/`, an app's detail page
(`/apps/:name`), its deploy detail (`/apps/:name/deploys/:id`), and
`/backups` — is a React SPA that talks to litehouse's existing JSON API
(`src/api.rs` — the same one the `lh` CLI and MCP server use), routed
client-side with `react-router-dom`. Only `/login` is still server-rendered
Askama HTML (see `src/ui.rs`). Every one of the SPA routes above is served
by the exact same HTML shell — see `src/ui.rs`'s `spa_shell` handler —
which mounts the React app and lets react-router decide what to render from
`location.pathname`.

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
- `react-router-dom` for client-side routing between the four pages (a
  `BrowserRouter` + `Routes` in `src/App.tsx`) — the server just serves the
  same shell for every SPA path (see `spa_shell` in `src/ui.rs`).
- `@tanstack/react-query` for data fetching/caching/polling (replaces the
  HTMX pages' `hx-trigger="every Ns"` polling with real invalidation +
  optimistic UI on start/stop/restart).
- `recharts` for the resource sparklines, `sonner` for toasts, `lucide-react`
  for icons, `class-variance-authority` + `tailwind-merge` for the couple of
  small component primitives in `src/components/`.

## Auth

The SPA never touches the admin token directly. It authenticates with the
same HttpOnly `litehouse_token` cookie set by `/login` (`credentials:
"include"` on every fetch — see `src/lib/api.ts`); `admin_auth_middleware`
(`src/auth.rs`) already accepts that cookie on `/api/*`, so no backend auth
changes were needed to reuse the JSON API from the browser.

## What's SPA-only vs. shared with the CLI

Most of `src/lib/api.ts` calls the same `/api/*` routes the CLI/MCP use.
A handful of endpoints exist only for this SPA (see `src/api.rs`), kept
separate rather than folded into the CLI-facing ones so their JSON
contract can evolve independently:

- `GET /api/apps/summary` — apps with **live** container state (not the
  cached DB column), best-effort URL, and latest deploy info in one call,
  for the dashboard's site-list view.
- `GET /api/apps/:name/summary` — the same live-state/URL treatment for one
  app's detail page, plus its image, repo, port, and custom domains.
- `GET /api/apps/:name/metrics?hours=24` — raw CPU/mem/disk samples scoped
  to one app, for its Resources sparklines. Same shape and `hours` clamp as
  `/api/metrics/server` below.
- `POST /api/apps/:name/restart` — stop+start under the app's lock, same as
  the dashboard's and app detail page's restart button.
- `GET /api/backups/catalog` — every catalogued backup artifact (app,
  object key, size, age), for the `/backups` page. Distinct from
  `/api/backups/status`'s today-only pass/fail summary.
- `GET /api/metrics/server?hours=24` — raw CPU/mem/disk samples for the
  dashboard's server resource sparklines.

Everything else on the app detail and deploy detail pages — start/stop,
redeploy (`POST /api/apps/:name/deploy`), the deploy list
(`GET /api/apps/:name/deploys`), env vars (`GET`/`POST
/api/apps/:name/env`), and the log tail (`GET /api/apps/:name/logs`) — reuse
the existing CLI/MCP-facing endpoints as-is. The env card only ever renders
the `key` of what `GET .../env` returns, matching the old HTMX page's
guarantee that a saved value is never shown again. Deploy detail has no
dedicated `GET .../deploys/:id` endpoint — the app's deploy list already
carries everything that page needs (including, at index 0, which deploy is
current), so it just looks the id up client-side.

## Migrating the rest of the admin UI

Done — every admin page is now React (`src/pages/Dashboard.tsx`,
`AppDetail.tsx`, `DeployDetail.tsx`, `Backups.tsx`), routed with
`react-router-dom` (`src/App.tsx`). Only `/login` is still server-rendered
Askama HTML. The pattern for any *new* admin page going forward: add
whatever JSON endpoint it needs (check `/api/*` first — most things already
exist), build the page under `src/pages/`, add a `<Route>` in `src/App.tsx`,
link to it with `<Link>`/`useParams` rather than `<a href>`, and — if it's
replacing a server-rendered page — delete that page's Askama template and
`src/ui.rs` handler once the React version works end-to-end, pointing its
route at `spa_shell` instead.
