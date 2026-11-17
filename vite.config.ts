import { defineConfig } from "vite";

// Tauri 권장 설정: 고정 포트, src-tauri 변경은 Vite가 감시하지 않음
export default defineConfig({
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**", "**/target/**"] },
  },
  envPrefix: ["VITE_", "TAURI_ENV_*"],
  build: {
    // macOS 10.15의 WKWebView(Safari 13)까지 지원
    target: "safari13",
    outDir: "dist",
    emptyOutDir: true,
  },
});
