#!/usr/bin/env bash
# 构建原生版 SSHL.app（release）。产物：target/release/bundle/SSHL.app（ad-hoc 签名）。
#
# 字体（JetBrainsMono Nerd Font）、Lucide 图标、应用图标都 include_bytes 进了二进制，
# bundle 里只需要可执行文件、Info.plist 和 Dock / Finder 用的 icon.icns。
# 依赖：rust-toolchain.toml 固定的 Rust（rustup 自动装）、Zig 0.17（libghostty-vt 构建用，brew install zig）。
#
# 用法: scripts/bundle.sh
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

command -v zig >/dev/null || { echo "✗ 缺少 Zig 0.17（brew install zig）：libghostty-vt 要用它编译"; exit 1; }

# 版本号：workspace Cargo.toml 的 [workspace.package] version
VERSION=$(awk '/^\[workspace\.package\]/{p=1;next} /^\[/{p=0} p && /^version *=/{gsub(/[" ]/,"",$0); split($0,a,"="); print a[2]; exit}' Cargo.toml)
[ -n "$VERSION" ] || { echo "✗ 读不到 Cargo.toml 的 workspace.package.version"; exit 1; }

# 正式构建：不带 devhook feature（那是无头测试用的按键注入口）
cargo build -p sshl --release

BIN="$ROOT/target/release/sshl"
APP="$ROOT/target/release/bundle/SSHL.app"
[ -x "$BIN" ] || { echo "✗ 没有构建产物: $BIN"; exit 1; }

# 最低系统版本跟随二进制实际的部署目标（LC_BUILD_VERSION 的 minos）
MIN_MACOS=$(otool -l "$BIN" | awk '/LC_BUILD_VERSION/{f=1} f && $1=="minos" && !done {print $2; done=1}')
MIN_MACOS=${MIN_MACOS:-11.0}

# 先删旧 bundle 再组装（覆盖会残留上一版的文件）
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/sshl"
cp "$ROOT/assets/icons/icon.icns" "$APP/Contents/Resources/icon.icns"
sed -e "s/__VERSION__/$VERSION/g" -e "s/__MIN_MACOS__/$MIN_MACOS/g" "$ROOT/assets/Info.plist" > "$APP/Contents/Info.plist"
printf 'APPL????' > "$APP/Contents/PkgInfo"
plutil -lint "$APP/Contents/Info.plist" >/dev/null

# ad-hoc 签名（发布产物不带个人证书）；本地安装由 scripts/deploy.sh 用 Developer ID 重签
codesign --force --sign - "$APP"
codesign --verify --strict "$APP"

echo "✓ ${APP}（版本 ${VERSION}，最低 macOS ${MIN_MACOS}，$(du -sh "$APP" | cut -f1)）"
