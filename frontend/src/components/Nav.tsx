import { NavLink } from "react-router-dom";
import { cn } from "../lib/cn";

// The server-rendered shell (see `spa_shell` in src/ui.rs) only carries the
// brand, theme toggle and sign-out — every page below it used to be reached
// by hunting for an inline link (backups lived inside a dashboard card;
// settings had no UI at all). This is the SPA's one persistent wayfinder,
// rendered once in App.tsx above <Routes>.
const links = [
  { to: "/", label: "apps", end: true },
  { to: "/backups", label: "backups", end: false },
  { to: "/settings", label: "settings", end: false },
];

export function Nav() {
  return (
    <nav className="-mt-4 mb-6 flex gap-1 border-b border-rule pb-2 text-xs uppercase tracking-wide">
      {links.map((l) => (
        <NavLink
          key={l.to}
          to={l.to}
          end={l.end}
          className={({ isActive }) =>
            cn(
              "px-2 py-1 font-semibold no-underline",
              isActive ? "bg-paper-3 text-ink" : "text-ink-3 hover:text-ink",
            )
          }
        >
          {l.label}
        </NavLink>
      ))}
    </nav>
  );
}
