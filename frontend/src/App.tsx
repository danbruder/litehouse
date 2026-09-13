import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { BrowserRouter, Routes, Route, Link } from "react-router-dom";
import { Toaster } from "sonner";
import { Dashboard } from "./pages/Dashboard";
import { AppDetail } from "./pages/AppDetail";
import { DeployDetail } from "./pages/DeployDetail";
import { Backups } from "./pages/Backups";
import { Settings } from "./pages/Settings";
import { Nav } from "./components/Nav";

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      // Actions elsewhere (the CLI, GitHub Actions deploys, nightly
      // restarts) change app state outside the browser, so treat every
      // query as immediately stale and lean on refetchInterval per-query
      // instead of a shared staleTime.
      staleTime: 0,
      retry: 1,
    },
  },
});

// Reached only by client-side navigation — the server serves the shell for
// the known SPA paths and 404s the rest (see `create_ui_router`).
function NotFound() {
  return (
    <div className="card">
      <span className="panel-label">not found</span>
      <p className="muted">That page doesn't exist.</p>
      <p>
        <Link to="/">&larr; all apps</Link>
      </p>
    </div>
  );
}

export function App() {
  return (
    <QueryClientProvider client={queryClient}>
      <BrowserRouter>
        <Nav />
        <Routes>
          <Route path="/" element={<Dashboard />} />
          <Route path="/apps/:name" element={<AppDetail />} />
          <Route path="/apps/:name/deploys/:deployId" element={<DeployDetail />} />
          <Route path="/backups" element={<Backups />} />
          <Route path="/settings" element={<Settings />} />
          <Route path="*" element={<NotFound />} />
        </Routes>
      </BrowserRouter>
      <Toaster
        position="bottom-right"
        theme={(document.documentElement.dataset.theme as "light" | "dark") ?? "light"}
        toastOptions={{
          style: {
            background: "var(--color-paper)",
            color: "var(--color-ink)",
            border: "1px solid var(--color-ink)",
          },
        }}
      />
    </QueryClientProvider>
  );
}
