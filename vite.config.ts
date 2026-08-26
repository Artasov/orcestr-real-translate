import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [react()],
  // The auth packages are local workspace links in development and CI. Without
  // explicit deduplication Rollup resolves their peer dependencies from the
  // auth workspace, producing a second React dispatcher in production only.
  resolve: {
    dedupe: ["react", "react-dom", "@tanstack/react-query"],
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
