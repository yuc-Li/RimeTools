#!/usr/bin/env bash
# 本地发布脚本：改版本号 → 跑测试 → 打包 → 打 tag → 推送。
#
#   ./scripts/release.sh 0.2.0            # 完整流程（最后会询问是否推送）
#   ./scripts/release.sh 0.2.0 --yes      # 不询问，直接推送
#   ./scripts/release.sh 0.2.0 --dry-run  # 只检查前置条件，不改任何文件
#
# 前置条件：
#   - 工作区干净，处于 main 分支且与 origin 同步
#   - CHANGELOG.md 已写好 `## [0.2.0] - YYYY-MM-DD` 小节（脚本不会替你写内容）
#
# 推送 tag 后，GitHub Actions 会自动构建并创建 Release（见 .github/workflows/release.yml）。

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

VERSION="${1:-}"
shift || true
DRY_RUN=false
ASSUME_YES=false
for arg in "$@"; do
  case "$arg" in
    --dry-run) DRY_RUN=true ;;
    --yes|-y) ASSUME_YES=true ;;
    *) echo "未知参数：$arg" >&2; exit 2 ;;
  esac
done

if [[ -z "$VERSION" ]]; then
  echo "用法：./scripts/release.sh <版本号> [--dry-run] [--yes]" >&2
  echo "例如：./scripts/release.sh 0.2.0" >&2
  exit 2
fi
if ! [[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "版本号格式应为 x.y.z（不带 v 前缀）：$VERSION" >&2
  exit 2
fi

TAG="v$VERSION"
say() { printf '\n==> %s\n' "$*"; }
fail() { printf '\n!! %s\n' "$*" >&2; exit 1; }

# ─────────────── 1. 前置检查 ───────────────
say "检查前置条件"

BRANCH="$(git branch --show-current)"
[[ "$BRANCH" == "main" ]] || fail "当前分支是 $BRANCH，请在 main 上发布"

[[ -z "$(git status --porcelain)" ]] || fail "工作区有未提交改动，请先提交"

git rev-parse --verify "origin/main" >/dev/null 2>&1 || fail "找不到 origin/main，请先 git push -u origin main"
LOCAL="$(git rev-parse HEAD)"
REMOTE="$(git rev-parse origin/main)"
[[ "$LOCAL" == "$REMOTE" ]] || fail "本地与 origin/main 不同步，请先 pull/push"

if git rev-parse --verify "refs/tags/$TAG" >/dev/null 2>&1; then
  fail "tag $TAG 已存在"
fi

grep -q "^## \[$VERSION\]" CHANGELOG.md \
  || fail "CHANGELOG.md 中没有 [$VERSION] 小节，请先补写好再发布"

say "CHANGELOG 中的 $VERSION 内容"
python3 scripts/release-notes.py "$VERSION"

if $DRY_RUN; then
  say "dry-run 结束：前置条件都满足，未做任何修改"
  exit 0
fi

# ─────────────── 2. 同步版本号 ───────────────
say "更新版本号到 $VERSION"

python3 - "$VERSION" <<'PY'
import json, re, sys, pathlib

version = sys.argv[1]

# package.json
p = pathlib.Path("package.json")
cfg = json.loads(p.read_text(encoding="utf-8"))
cfg["version"] = version
p.write_text(json.dumps(cfg, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")

# tauri.conf.json
q = pathlib.Path("src-tauri/tauri.conf.json")
cfg = json.loads(q.read_text(encoding="utf-8"))
cfg["version"] = version
q.write_text(json.dumps(cfg, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")

# workspace Cargo.toml（只改 [workspace.package] 里的 version）
r = pathlib.Path("src-tauri/Cargo.toml")
text = r.read_text(encoding="utf-8")
new, n = re.subn(r'(?m)^version = "[^"]+"', f'version = "{version}"', text, count=1)
if n != 1:
    raise SystemExit("未能定位 Cargo.toml 的 version 字段")
r.write_text(new, encoding="utf-8")

print(f"  package.json / tauri.conf.json / Cargo.toml → {version}")
PY

# Cargo.lock 会因工作区版本变化而更新
if [[ -d "$ROOT/.cargo-home" && -z "${CARGO_HOME:-}" ]]; then
  export CARGO_HOME="$ROOT/.cargo-home"
  export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/.cargo-target/release}"
fi
export PATH="$HOME/.cargo/bin:$PATH"
( cd src-tauri && cargo update --workspace --offline 2>/dev/null || cargo update --workspace ) >/dev/null

# ─────────────── 3. 测试与打包 ───────────────
say "运行测试"
( cd src-tauri && cargo test --workspace )

say "构建前端与 release 二进制"
npm run build
( cd src-tauri && cargo build -p rimetools --release --features custom-protocol )

say "打包"
./scripts/package.sh --no-build

# ─────────────── 4. 提交与打 tag ───────────────
say "提交版本号变更"
git add -A
git commit -m "release: $TAG"

git tag -a "$TAG" -m "RimeTools $TAG"
say "已创建 tag $TAG"

# ─────────────── 5. 推送 ───────────────
if ! $ASSUME_YES; then
  echo ""
  read -r -p "推送到 origin（含 tag，会触发 GitHub Release）？[y/N] " reply
  [[ "$reply" =~ ^[Yy]$ ]] || { echo "已取消推送。本地提交与 tag 已创建，可稍后手动推送。"; exit 0; }
fi

git push origin main
git push origin "$TAG"

say "完成：$TAG 已推送，GitHub Actions 将构建并创建 Release"
echo "  查看进度：https://github.com/yuc-Li/RimeTools/actions"