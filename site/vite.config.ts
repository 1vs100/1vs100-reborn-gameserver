import { defineConfig, loadEnv } from "vite";
import react from "@vitejs/plugin-react";

// In production nginx serves the site and proxies `/status` (deployment/nginx/). For `pnpm dev` and
// `pnpm preview` this config proxies it instead. Override the target with STATUS_URL.
export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, ".", "STATUS_");
  const proxy = {
    "/status": {
      target: env.STATUS_URL || "http://127.0.0.1:8082",
      changeOrigin: true,
    },
  };
  return {
    plugins: [react()],
    server: { proxy },
    preview: { proxy, allowedHosts: true },
  };
});
