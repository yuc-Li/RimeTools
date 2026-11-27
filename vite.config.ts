import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { fileURLToPath, URL } from "node:url";

// Tauri 期望固定端口，且不清屏以便看到 Rust 侧日志
export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: { "@": fileURLToPath(new URL("./src", import.meta.url)) },
  },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      // 必须忽略 Rust 构建目录：workspace 里 .cargo-target 下有几万个文件，
      // 让 Vite 监听它会耗尽系统 inotify 配额（ENOSPC），dev server 直接崩溃、
      // 页面白屏。这类"看代码没问题但界面不出来"的坑排查成本很高。
      ignored: [
        "**/.cargo-target/**",
        "**/.cargo-home/**",
        "**/.toolchain/**",
        "**/target/**",
        "**/src-tauri/**",
        "**/.verify/**",
      ],
    },
  },
  build: {
    // Tauri 使用固定 WebView 版本，可按需下调
    target: "es2022",
    sourcemap: false,
  },
});