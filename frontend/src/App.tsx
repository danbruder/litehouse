import { useEffect, useState } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { BrowserRouter, Routes, Route, Link, useLocation } from "react-router-dom";
import { Toaster } from "sonner";
import { Dashboard } from "./pages/Dashboard";
import { AppDetail } from "./pages/AppDetail";
import { DeployDetail } from "./pages/DeployDetail";
import { Backups } from "./pages/Backups";
import { Settings } from "./pages/Settings";
import { Shell } from "./components/Shell";
import { Page, PageHeader, Muted } from "./components/ui";

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
    <Page>
      <PageHeader title="Not found" />
      <Muted>
        That page doesn't exist. <Link to="/">Back to apps</Link>
      </Muted>
    </Page>
  );
}

function useIsPhone() {
  const query = "(max-width: 767px)";
  const [phone, setPhone] = useState(() => window.matchMedia(query).matches);
  useEffect(() => {
    const mq = window.matchMedia(query);
    const on = () => setPhone(mq.matches);
    mq.addEventListener("change", on);
    return () => mq.removeEventListener("change", on);
  }, []);
  return phone;
}

function Routed() {
  const { pathname } = useLocation();
  const phone = useIsPhone();
  // Each page starts at the top, as a full page load would.
  useEffect(() => window.scrollTo(0, 0), [pathname]);

  return (
    <>
      <Shell pathname={pathname}>
        <Routes>
          <Route path="/" element={<Dashboard />} />
          <Route path="/apps/:name" element={<AppDetail />} />
          <Route path="/apps/:name/deploys/:deployId" element={<DeployDetail />} />
          <Route path="/backups" element={<Backups />} />
          <Route path="/settings" element={<Settings />} />
          <Route path="*" element={<NotFound />} />
        </Routes>
      </Shell>
      <Toaster
        position={phone ? "top-center" : "bottom-right"}
        theme={(document.documentElement.dataset.theme as "light" | "dark") ?? "light"}
        toastOptions={{
          style: {
            background: "var(--color-paper)",
            color: "var(--color-ink)",
            border: "1px solid var(--color-ink)",
            borderRadius: 0,
            fontFamily: "var(--font-sans)",
          },
        }}
      />
    </>
  );
}

export function App() {
  return (
    <QueryClientProvider client={queryClient}>
      <BrowserRouter>
        <Routed />
      </BrowserRouter>
    </QueryClientProvider>
  );
}
