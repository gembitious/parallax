import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
// @ts-expect-error type error without @types/node package
import process from "node:process";
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(() => ({
  plugins: [react()],
  // 앱 파일은 로컬에서 읽으므로 내려받는 비용이 없다. 메인 청크(React + CodeMirror 코어)는 600kB 남짓이다.
  // 언어 모듈은 따로 나뉘어 필요할 때 불러온다.
  build: { chunkSizeWarningLimit: 1000 },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
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
      // 3. tell Vite to ignore watching `src-tauri`
      // 빌드 산출물과 벤치 참조 저장소(git/git·vscode, 수십만 파일)도 감시하지 않는다.
      ignored: ["**/src-tauri/**", "**/target/**", "**/bench/repos/**"],
    },
  },
}));
