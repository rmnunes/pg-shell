import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri expects the dev server on a fixed port
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: "127.0.0.1",
    watch: {
      // don't watch Rust/target
      ignored: ["**/src-tauri/**", "**/target/**"],
    },
  },
  envPrefix: ["VITE_", "TAURI_ENV_"],
  build: {
    target: "esnext",
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
    minify: !process.env.TAURI_ENV_DEBUG ? "esbuild" : false,
    chunkSizeWarningLimit: 2500,
    rollupOptions: {
      output: {
        manualChunks: {
          // Keep Monaco in its own cacheable chunk; app code stays small and
          // unaffected by editor updates.
          monaco: ["monaco-editor", "@monaco-editor/react"],
        },
      },
    },
  },
});
