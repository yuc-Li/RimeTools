#!/usr/bin/env bash
# RimeTools 启动脚本（Linux）
#
#   ./scripts/dev.sh          # 开发模式：Vite 热更新 + Tauri（改前端即时生效）
#   ./scripts/dev.sh --prod   # 独立模式：前端资源嵌进二进制，不依赖 dev server（启动快）
#
# 原理：Tauri 的 is_dev() = !cfg!(feature="custom-protocol")。
#   未启用 → 程序去连 tauri.conf.json 的 devUrl(127.0.0.1:1420)，必须先起 Vite；
#   启用   → 前端从二进制内嵌的 dist/ 加载，单文件即可运行。

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TAURI_DIR="$ROOT/src-tauri"

# 让 cargo 可用（用户级安装）
[[ -d "$HOME/.cargo/bin" ]] && export PATH="$HOME/.cargo/bin:$PATH"

# 仅当检测到本仓库的隔离缓存目录时才使用它（AI 沙箱/只读家目录场景）；
# 你自己机器上正常跑就走默认的 ~/.cargo 与 target/，无需额外等待重新编译。
if [[ -d "$ROOT/.cargo-home" && -z "${CARGO_HOME:-}" ]]; then
  export CARGO_HOME="$ROOT/.cargo-home"
  export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/.cargo-target/app}"
fi

MODE="${1:-dev}"

BIN="${CARGO_TARGET_DIR:-$TAURI_DIR/target}/debug/rimetools"

if [[ "$MODE" == "--prod" ]]; then
  cd "$ROOT"
  echo "==> 构建前端 → dist/"
  npm run build

  echo "==> 构建二进制（custom-protocol，内嵌前端）"
  cd "$TAURI_DIR"
  cargo build -p rimetools --features custom-protocol

  echo "==> 启动"
  exec "$BIN"
fi

# ── 开发模式 ──
cd "$ROOT"
if [[ ! -d node_modules ]]; then
  echo "==> 安装前端依赖"
  npm install --no-audit --no-fund
fi

echo "==> 启动 Vite (127.0.0.1:1420)"
npm run dev &
VITE_PID=$!
trap 'kill $VITE_PID 2>/dev/null || true' EXIT

for _ in $(seq 1 120); do
  if (exec 3<>/dev/tcp/127.0.0.1/1420) 2>/dev/null; then
    exec 3<&- 3>&-
    echo "==> Vite 就绪"
    break
  fi
  kill -0 "$VITE_PID" 2>/dev/null || { echo "!! Vite 启动失败" >&2; exit 1; }
  sleep 0.5
done

echo "==> 启动 Tauri（首次编译较久，请耐心）"
cd "$TAURI_DIR"
exec cargo run -p rimetools