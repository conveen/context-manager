/// <reference types="vitest/config" />
import { svelte } from "@sveltejs/vite-plugin-svelte";
import { svelteTesting } from "@testing-library/svelte/vite";
import { defineConfig } from "vite";

const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
    // svelteTesting is a no-op outside Vitest runs; it fixes module resolution
    // (browser conditions) for Svelte 5 under jsdom and auto-cleans up the DOM
    // between tests.
    plugins: [svelte(), svelteTesting()],
    clearScreen: false,
    server: {
        port: 1420,
        strictPort: true,
        host: host || false,
        hmr: host
            ? {
                  protocol: "ws",
                  host,
                  port: 1421,
              }
            : undefined,
        watch: {
            ignored: ["**/src-tauri/**"],
        },
    },
    test: {
        environment: "jsdom",
        include: ["src/**/*.test.ts"],
    },
});
