import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";

// `base: "./"` is required by Tauri: the packaged webview serves
// `frontendDist` from a custom protocol (`tauri://localhost`), so an
// absolute `/assets/...` path would resolve against the wrong origin
// and the page renders blank.
export default defineConfig({
  plugins: [vue()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      ignored: ["**/src-tauri/**", "**/target/**"],
    },
  },
  base: "./",
  envPrefix: ["VITE_", "TAURI_"],
});
