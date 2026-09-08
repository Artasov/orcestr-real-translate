import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [react()],
  // The auth packages are local workspace links in development and CI. Without
  // explicit deduplication their peer dependencies resolve from the auth
  // workspace, creating separate React dispatchers and UI overlay contexts.
  resolve: {
    dedupe: ["react", "react-dom", "@tanstack/react-query", "@orcestr/ui"],
  },
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
});
