import { sveltekit } from "@sveltejs/kit/vite";
import { defineConfig } from "vite";

export default defineConfig({
  plugins: [sveltekit()],
  server: {
    port: 1421,
    strictPort: true,
    proxy: {
      "/api": {
        target: process.env.RELAY_API_URL ?? "http://127.0.0.1:3000",
        changeOrigin: true,
      },
    },
  },
  build: {
    target: "esnext",
    minify: "esbuild",
    sourcemap: false,
    cssMinify: "lightningcss",
  },
});
