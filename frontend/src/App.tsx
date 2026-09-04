import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { Toaster } from "sonner";
import { Dashboard } from "./pages/Dashboard";

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

export function App() {
  return (
    <QueryClientProvider client={queryClient}>
      <Dashboard />
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
