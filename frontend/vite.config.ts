import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// The SPA is built once (locally or in CI, never on the server — litehouse
// never builds anything at runtime) and its output is committed into
// src/ui/spa/, where litehouse's Rust server embeds it with `include_str!`
// and serves it as the admin dashboard. See src/ui.rs's `spa` module.
//
// Fixed, non-hashed filenames are deliberate: `include_str!` paths are
// resolved at Rust compile time, so the Rust side needs to know the exact
// asset filenames ahead of time rather than reading a Vite-generated
// manifest.
export default defineConfig({
  plugins: [react()],
  build: {
    outDir: "../src/ui/spa",
    emptyOutDir: true,
    rollupOptions: {
      output: {
        entryFileNames: "spa.js",
        chunkFileNames: "spa-[name].js",
        assetFileNames: (info) =>
          info.name?.endsWith(".css") ? "spa.css" : "spa-[name][extname]",
      },
    },
  },
  server: {
    proxy: {
      // `lh serve` (LITEHOUSE_LOCAL_DEV / debug build) listens on
      // localhost:3030 — see src/config.rs. `npm run dev` proxies both the
      // JSON API and the cookie-auth endpoints so the SPA dev server talks
      // to a real, running litehouse-server with no CORS setup.
      "/api": "http://localhost:3030",
      "/login": "http://localhost:3030",
      "/logout": "http://localhost:3030",
      "/assets/styles.css": "http://localhost:3030",
      "/assets/htmx.min.js": "http://localhost:3030",
    },
  },
});
