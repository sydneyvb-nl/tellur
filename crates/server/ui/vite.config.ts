/// <reference types="vitest" />
import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// The hub serves these assets under /app, so all asset URLs are /app-prefixed.
export default defineConfig({
  base: "/app/",
  plugins: [svelte()],
  build: { outDir: "dist", emptyOutDir: true },
  server: {
    // `pnpm dev` proxies the JSON API to a locally running hub.
    proxy: {
      "/v1": "http://127.0.0.1:4920",
      "/auth": "http://127.0.0.1:4920",
    },
  },
  test: {
    environment: "node",
    include: ["src/**/*.test.ts"],
  },
});
