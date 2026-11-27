#!/usr/bin/env bash
# RimeTools 打包脚本（Linux：deb + 便携 tarball）
#
#   ./scripts/package.sh            # 构建 release 并打包
#   ./scripts/package.sh --no-build # 复用已有 release 二进制，只打包
#
# 产物：dist-pkg/rimetools_<版本>_<架构>.deb
#       dist-pkg/rimetools-<版本>-<架构>.tar.gz（便携，含 .desktop 与安装脚本）
#
# 说明：不依赖 tauri-cli。deb 用 dpkg-deb 直接组装，便于看清包内容。

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

VERSION="$(node -p "require('./package.json').version" 2>/dev/null || echo "0.1.0")"
ARCH="$(dpkg --print-architecture)"
OUT="$ROOT/dist-pkg"
APP="rimetools"

[[ -d "$HOME/.cargo/bin" ]] && export PATH="$HOME/.cargo/bin:$PATH"
if [[ -d "$ROOT/.cargo-home" && -z "${CARGO_HOME:-}" ]]; then
  export CARGO_HOME="$ROOT/.cargo-home"
  export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/.cargo-target/release}"
fi
BIN="${CARGO_TARGET_DIR:-$ROOT/src-tauri/target}/release/$APP"
# CARGO_TARGET_DIR 会再套一层 profile 目录
[[ -f "$BIN" ]] || BIN="${CARGO_TARGET_DIR:-$ROOT/src-tauri/target}/release/release/$APP"

if [[ "${1:-}" != "--no-build" ]]; then
  echo "==> 构建前端"
  npm run build
  echo "==> 构建 release 二进制"
  ( cd src-tauri && cargo build -p "$APP" --release --features custom-protocol )
fi

if [[ ! -f "$BIN" ]]; then
  echo "!! 找不到 release 二进制：$BIN" >&2
  exit 1
fi
echo "==> 使用二进制 $BIN ($(du -h "$BIN" | cut -f1))"

rm -rf "$OUT"
mkdir -p "$OUT"

# ───────────────────────── deb ─────────────────────────
DEBROOT="$OUT/deb-root"
mkdir -p "$DEBROOT/DEBIAN" \
         "$DEBROOT/usr/bin" \
         "$DEBROOT/usr/share/applications" \
         "$DEBROOT/usr/share/doc/$APP"

install -m 0755 "$BIN" "$DEBROOT/usr/bin/$APP"
for spec in 32:32x32 48:48x48 64:64x64 128:128x128 256:256x256 512:icon; do
  px="${spec%%:*}"; src="${spec##*:}"
  install -d "$DEBROOT/usr/share/icons/hicolor/${px}x${px}/apps"
  install -m 0644 "src-tauri/icons/${src}.png" "$DEBROOT/usr/share/icons/hicolor/${px}x${px}/apps/$APP.png"
done
install -m 0644 README.md "$DEBROOT/usr/share/doc/$APP/README.md"

DESKTOP_FILE="$DEBROOT/usr/share/applications/$APP.desktop"
cat > "$DESKTOP_FILE" <<'EOF'
[Desktop Entry]
Type=Application
Name=RimeTools
GenericName=Rime Dictionary Manager
Comment=Manage Rime dictionaries, backups and deployment
Exec=rimetools
Icon=rimetools
Terminal=false
Categories=Utility;
Keywords=rime;fcitx;weasel;input;dictionary;
StartupNotify=true
StartupWMClass=rimetools
EOF

INSTALLED_SIZE=$(du -sk "$DEBROOT/usr" | cut -f1)
cat > "$DEBROOT/DEBIAN/control" <<EOF
Package: $APP
Version: $VERSION
Section: utils
Priority: optional
Architecture: $ARCH
Installed-Size: $INSTALLED_SIZE
Maintainer: RimeTools <dev@rimetools.local>
Depends: libwebkit2gtk-4.1-0, libgtk-3-0 | libgtk-3-0t64, librime1 | librime1t64
Recommends: fcitx5-rime | ibus-rime
Description: Cross-platform Rime dictionary manager
 Manage personal Rime dictionaries on Windows (Weasel) and Linux (fcitx5-rime):
 browse/edit entries with code suggestions, automatic backups with diff preview,
 and one-click redeployment. Files are written in Rime's native format with
 byte-level round-trip fidelity.
EOF

DEB="$OUT/${APP}_${VERSION}_${ARCH}.deb"
dpkg-deb --root-owner-group --build "$DEBROOT" "$DEB" >/dev/null
echo "==> deb: $DEB ($(du -h "$DEB" | cut -f1))"

# ───────────────────────── 便携 tarball ─────────────────────────
PORTABLE="$OUT/${APP}-${VERSION}-${ARCH}"
mkdir -p "$PORTABLE"
install -m 0755 "$BIN" "$PORTABLE/$APP"
cp "$DESKTOP_FILE" "$PORTABLE/$APP.desktop"
cp src-tauri/icons/icon.png "$PORTABLE/$APP.png"
cp README.md "$PORTABLE/README.md"
cat > "$PORTABLE/install.sh" <<'EOF'
#!/usr/bin/env bash
# 便携版安装：把程序与桌面入口装到用户目录（无需 root）
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN_DIR="$HOME/.local/bin"
APP_DIR="$HOME/.local/share/applications"
ICON_DIR="$HOME/.local/share/icons/hicolor/512x512/apps"
mkdir -p "$BIN_DIR" "$APP_DIR" "$ICON_DIR"
install -m 0755 "$HERE/rimetools" "$BIN_DIR/rimetools"
install -m 0644 "$HERE/rimetools.png" "$ICON_DIR/rimetools.png"
sed "s|^Exec=.*|Exec=$BIN_DIR/rimetools|" "$HERE/rimetools.desktop" > "$APP_DIR/rimetools.desktop"
update-desktop-database "$APP_DIR" 2>/dev/null || true
gtk-update-icon-cache -f -t "$HOME/.local/share/icons/hicolor" 2>/dev/null || true
echo "已安装到 $BIN_DIR/rimetools"
echo "若 $BIN_DIR 不在 PATH 中，请自行加入；也可直接从 $BIN_DIR/rimetools 启动。"
EOF
chmod +x "$PORTABLE/install.sh"
TAR="$OUT/${APP}-${VERSION}-${ARCH}.tar.gz"
tar -czf "$TAR" -C "$OUT" "$(basename "$PORTABLE")"
echo "==> 便携包: $TAR ($(du -h "$TAR" | cut -f1))"

echo ""
echo "打包完成，产物在 $OUT："
ls -lh "$OUT" | grep -vE "^total|^d" | awk '{print "  " $9 "  (" $5 ")"}'