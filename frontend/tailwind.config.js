/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  // The base reset/typography/tokens all come from litehouse's existing
  // src/ui/styles.css (linked in the shell before spa.css) — Tailwind is
  // layered on top for layout utilities and new components, not a second
  // reset. See that file's ":root" block for where these vars come from.
  corePlugins: { preflight: false },
  theme: {
    extend: {
      colors: {
        paper: "var(--color-paper)",
        "paper-2": "var(--color-paper-2)",
        "paper-3": "var(--color-paper-3)",
        ink: "var(--color-ink)",
        "ink-2": "var(--color-ink-2)",
        "ink-3": "var(--color-ink-3)",
        rule: "var(--color-rule)",
        lime: "var(--color-lime)",
        "on-lime": "var(--on-lime)",
        signal: "var(--color-signal)",
        good: "var(--color-good)",
        warn: "var(--color-warn)",
        bad: "var(--color-bad)",
      },
      fontFamily: {
        display: ["var(--font-display)"],
        sans: ["var(--font-sans)"],
        mono: ["var(--font-mono)"],
      },
      borderRadius: {
        none: "0",
        DEFAULT: "0",
        sm: "0",
        md: "0",
        lg: "0",
        xl: "0",
        full: "0",
      },
    },
  },
  plugins: [],
};
