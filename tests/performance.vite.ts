import { defineConfig, mergeConfig } from "vite";
import base from "../vite.config";

export default defineConfig(async (environment) =>
  mergeConfig(await base(environment), {
    build: { outDir: "dist-perf", rollupOptions: { input: "tests/ui/performance.html" } },
  }),
);
