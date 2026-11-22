import { useEffect, useState, type ReactNode } from "react";
import { Link, NavLink } from "react-router-dom";
import { Boxes, DatabaseBackup, LogOut, Moon, Settings, Sun } from "lucide-react";
import { cn } from "../lib/cn";

// The SPA's chrome. Phones get a compact sticky top bar plus a bottom tab
// bar in thumb reach (the shell is often launched from the home screen, so
// it respects the safe-area insets); wider screens get one top bar with the
// nav inline. Sign-out stays a real form POST to /logout so the server
// clears the HttpOnly cookie.

const links = [
  { to: "/", label: "Apps", icon: Boxes, end: false, match: (p: string) => p === "/" || p.startsWith("/apps") },
  { to: "/backups", label: "Backups", icon: DatabaseBackup, end: false, match: (p: string) => p.startsWith("/backups") },
  { to: "/settings", label: "Settings", icon: Settings, end: false, match: (p: string) => p.startsWith("/settings") },
];

function useTheme() {
  const [theme, setTheme] = useState<"light" | "dark">(
    (document.documentElement.dataset.theme as "light" | "dark") ?? "light",
  );
  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    // Keep the browser chrome (iOS status bar, Android address bar) in
    // step with the in-app theme rather than the OS one.
    const color = getComputedStyle(document.documentElement).getPropertyValue("--color-paper").trim();
    document.querySelectorAll('meta[name="theme-color"]').forEach((m) => {
      m.setAttribute("content", color);
      m.removeAttribute("media");
    });
  }, [theme]);
  const toggle = () => {
    const next = theme === "dark" ? "light" : "dark";
    localStorage.theme = next;
    setTheme(next);
  };
  return { theme, toggle };
}

function Brand() {
  return (
    <Link to="/" className="row-link inline-flex items-center gap-1.5 no-underline" aria-label="litehouse home">
      <svg className="h-[18px] w-[18px]" viewBox="0 0 100 100" aria-hidden="true">
        <polygon points="60,24 96,6 96,22" fill="#ffd54a" />
        <polygon points="28,94 72,94 62,84 38,84" fill="#4a4a4a" />
        <polygon points="40,84 60,84 55,30 45,30" fill="#f4f1e8" />
        <polygon points="42.5,62 57.5,62 56,50 44,50" fill="#d94f4f" />
        <rect x="42" y="18" width="16" height="12" fill="#2b2b2b" />
        <polygon points="40,18 60,18 50,6" fill="#d94f4f" />
      </svg>
      <span className="font-display text-[1.05rem] font-bold tracking-tight text-ink">
        litehouse<span className="text-lime">_</span>
      </span>
    </Link>
  );
}

const iconBtn =
  "inline-flex h-10 w-10 items-center justify-center border-0 bg-transparent p-0 text-ink-2 hover:bg-paper-3 hover:text-ink md:h-9 md:w-9";

export function Shell({ children, pathname }: { children: ReactNode; pathname: string }) {
  const { theme, toggle } = useTheme();

  return (
    <div className="flex min-h-screen flex-col">
      <div className="pt-safe sticky top-0 z-30 border-0 border-b border-solid border-ink bg-paper">
        <div className="mx-auto flex h-14 w-full max-w-5xl items-center justify-between gap-4 px-4 md:px-8">
          <div className="flex items-center gap-8">
            <Brand />
            <nav className="hidden items-center gap-1 md:flex" aria-label="Main">
              {links.map((l) => (
                <NavLink
                  key={l.to}
                  to={l.to}
                  className={cn(
                    "row-link inline-flex h-9 items-center px-3 text-xs font-semibold uppercase tracking-wider",
                    l.match(pathname) ? "!bg-paper-3 !text-ink" : "!text-ink-3 hover:!text-ink",
                  )}
                >
                  {l.label}
                </NavLink>
              ))}
            </nav>
          </div>
          <div className="flex items-center gap-1">
            <button
              type="button"
              className={iconBtn}
              onClick={toggle}
              aria-label={theme === "dark" ? "Switch to light theme" : "Switch to dark theme"}
              title={theme === "dark" ? "Light theme" : "Dark theme"}
            >
              {theme === "dark" ? <Sun size={17} /> : <Moon size={17} />}
            </button>
            <form method="post" action="/logout" className="m-0 inline">
              <button type="submit" className={iconBtn} aria-label="Sign out" title="Sign out">
                <LogOut size={17} />
              </button>
            </form>
          </div>
        </div>
      </div>

      <main className="m-0 flex-1 p-0" style={{ maxWidth: "none" }}>
        {children}
      </main>

      <nav
        aria-label="Main"
        className="pb-safe fixed inset-x-0 bottom-0 z-30 grid grid-cols-3 border-0 border-t border-solid border-ink bg-paper md:hidden"
      >
        {links.map((l) => {
          const active = l.match(pathname);
          const Icon = l.icon;
          return (
            <NavLink
              key={l.to}
              to={l.to}
              className={cn(
                "row-link relative flex h-16 flex-col items-center justify-center gap-1 text-[10px] font-semibold uppercase tracking-wider",
                active ? "!text-ink" : "!text-ink-3",
              )}
            >
              {active && <span className="absolute inset-x-6 top-0 h-[3px] bg-lime" aria-hidden />}
              <Icon size={20} strokeWidth={active ? 2.25 : 1.75} />
              {l.label}
            </NavLink>
          );
        })}
      </nav>
    </div>
  );
}
