import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import path from "path";

// Tauri dev server 固定端口，与 src-tauri/tauri.conf.json 的 devUrl 保持一致
export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  server: {
    host: "0.0.0.0",
    port: 5173,
    strictPort: true,
    allowedHosts: true,
  },
  clearScreen: false,
  envPrefix: ["VITE_", "TAURI_"],
  build: {
    target: "chrome105",
    minify: "esbuild",
    sourcemap: false,
  },
});
