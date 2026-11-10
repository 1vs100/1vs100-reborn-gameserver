import { defineConfig, loadEnv } from "vite";
import react from "@vitejs/plugin-react";

// The admin API lives on the gameserver; the browser only ever talks to the
// frontend's own origin and `/admin` is proxied through (nginx in production,
// see deployment/nginx/, this config for `pnpm dev` / `pnpm preview`). Same-origin means
// the Rust side needs no CORS layer. Override the target with ADMIN_URL.
export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, ".", "ADMIN_");
  const proxy = {
    "/admin": {
      target: env.ADMIN_URL || "http://127.0.0.1:8081",
      changeOrigin: true,
      // admin operations are quick; keep headroom for slow ones.
      timeout: 120000,
      proxyTimeout: 120000,
    },
  };
  return {
    plugins: [react()],
    server: { proxy },
    preview: { proxy, allowedHosts: true },
  };
});
