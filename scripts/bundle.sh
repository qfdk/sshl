#!/usr/bin/env bash
# 构建原生版 SSHL.app（release）。产物：target/release/bundle/SSHL.app（ad-hoc 签名）。
#
# 字体（JetBrainsMono Nerd Font）、Lucide 图标、应用图标都 include_bytes 进了二进制，
# bundle 里只需要可执行文件、Info.plist、Dock / Finder 用的 icon.icns，外加命令行 sshl 和访达工具栏按钮。
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

# 正式构建：不带 devhook feature（那是无头测试用的按键注入口）。open-in-sshl 是访达工具栏按钮（crates/finder-button）
cargo build -p sshl -p sshl-finder-button --release

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

# 命令行 sshl（设置里链接到 /usr/local/bin）
mkdir -p "$APP/Contents/Resources/bin"
cp "$ROOT/assets/bin/sshl" "$APP/Contents/Resources/bin/sshl"

# 访达工具栏按钮「在 SSHL 中打开」：按住 ⌘ 拖到访达工具栏上（设置里有「在访达中显示」）
HELPER="$APP/Contents/Helpers/Open in SSHL.app"
mkdir -p "$HELPER/Contents/MacOS" "$HELPER/Contents/Resources"
cp "$ROOT/target/release/open-in-sshl" "$HELPER/Contents/MacOS/open-in-sshl"
cp "$ROOT/assets/finder-button/icon.icns" "$HELPER/Contents/Resources/icon.icns"
cp -R "$ROOT/assets/finder-button/zh-Hans.lproj" "$HELPER/Contents/Resources/"
sed -e "s/__VERSION__/$VERSION/g" -e "s/__MIN_MACOS__/$MIN_MACOS/g" "$ROOT/assets/finder-button/Info.plist" > "$HELPER/Contents/Info.plist"
printf 'APPL????' > "$HELPER/Contents/PkgInfo"
plutil -lint "$HELPER/Contents/Info.plist" >/dev/null

# 先 ad-hoc 签名；本地安装由 scripts/deploy.sh、发布包由 scripts/release.sh 用 Developer ID 重签
source "$ROOT/scripts/sign.sh"
sign_app "$APP" -

echo "✓ ${APP}（版本 ${VERSION}，最低 macOS ${MIN_MACOS}，$(du -sh "$APP" | cut -f1)）"
