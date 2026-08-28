import { sveltekit } from "@sveltejs/kit/vite";
import { defineConfig } from "vite";

const basePath = process.env.RELAY_BASE_PATH ?? "";
const apiTarget = process.env.RELAY_API_URL ?? "http://127.0.0.1:3000";

function apiProxy(prefix: string) {
  return {
    target: apiTarget,
    changeOrigin: true,
    rewrite: (path: string) => (prefix ? path.replace(new RegExp("^" + prefix), "") : path),
  };
}

export default defineConfig({
  plugins: [sveltekit()],
  server: {
    host: "0.0.0.0",
    port: 1421,
    strictPort: true,
    allowedHosts: true,
    hmr: { protocol: "wss", clientPort: 443 },
    proxy: {
      "/api": apiProxy(""),
      ...(basePath ? { [`${basePath}/api`]: apiProxy(basePath) } : {}),
    },
  },
  build: {
    target: "esnext",
    minify: "esbuild",
    sourcemap: false,
    cssMinify: "lightningcss",
  },
});
