import { svelte } from "@sveltejs/vite-plugin-svelte";
import { fileURLToPath } from "node:url";
import { defineConfig } from "vitest/config";

export default defineConfig({
  plugins: [svelte({ compilerOptions: { sourcemap: false }, hot: false, emitCss: false })],
  resolve: {
    alias: {
      $lib: fileURLToPath(new URL("./src/lib", import.meta.url)),
      "$app/navigation": fileURLToPath(new URL("./src/__tests__/$app-navigation-mock.ts", import.meta.url)),
    },
    conditions: ["browser"],
  },
  test: {
    globals: true,
    environment: "jsdom",
    include: ["src/**/*.{test,spec}.{ts,js}"],
    setupFiles: ["src/__tests__/setup.ts"],
    // Sandbox-freundlich: ein einzelner Fork. Der Parent (vite-Server) bleibt
    // klein (~300MB), der Fork-Worker bekommt fast das gesamte cgroup-Budget.
    pool: "forks",
    poolOptions: {
      forks: {
        singleFork: true,
        minForks: 1,
        maxForks: 1,
        execArgv: ["--max-old-space-size=7168"],
      },
    },
    testTimeout: 30000,
    css: false,
    coverage: {
      provider: "v8",
      include: ["src/lib/**/*.{ts,svelte}"],
      exclude: ["src/lib/workers/**"],
      thresholds: {
        lines: 80,
        functions: 70,
      },
    },
  },
});
