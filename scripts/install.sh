#!/bin/sh
# 一行安装 / 升级 SSHL：下载最新发布包，装到 /Applications，清掉隔离标记。
#
#   curl -fsSL https://raw.githubusercontent.com/qfdk/sshl/main/scripts/install.sh | sh
#
# 为什么用脚本：发布包只有 ad-hoc 签名、没有 Apple 公证（公证会把开发者实名写进应用）。
# 浏览器下载的文件带隔离标记，系统会报「已损坏」；curl 下载的不带，装完直接能开。
# 装到别处：SSHL_INSTALL_DIR=~/Applications sh install.sh
set -eu

REPO="qfdk/sshl"
DEST_DIR="${SSHL_INSTALL_DIR:-/Applications}"
DEST="$DEST_DIR/SSHL.app"

[ "$(uname -s)" = "Darwin" ] || { echo "✗ SSHL 只支持 macOS" >&2; exit 1; }

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

URL="$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" | grep -o '"browser_download_url": *"[^"]*macos\.zip"' | head -1 | cut -d'"' -f4)"
[ -n "$URL" ] || { echo "✗ 没找到最新发布包（https://github.com/$REPO/releases）" >&2; exit 1; }

echo "下载 $URL"
curl -fL --progress-bar -o "$TMP/sshl.zip" "$URL"
ditto -x -k "$TMP/sshl.zip" "$TMP/unpacked"
[ -d "$TMP/unpacked/SSHL.app" ] || { echo "✗ 发布包里没有 SSHL.app" >&2; exit 1; }

# 关窗只是隐藏、进程常驻：换文件前先退出正在跑的
if [ -d "$DEST" ]; then
  osascript -e 'quit app "SSHL"' >/dev/null 2>&1 || true
  pkill -f "$DEST/Contents/MacOS/sshl" 2>/dev/null || true
  rm -rf "$DEST"
fi
mkdir -p "$DEST_DIR"
ditto "$TMP/unpacked/SSHL.app" "$DEST"
xattr -cr "$DEST"

VERSION="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$DEST/Contents/Info.plist" 2>/dev/null || echo '?')"
echo "✓ SSHL $VERSION 已装到 $DEST"
echo "  连局域网主机时系统会问「本地网络」权限，请允许（每次升级后会再问一次）。"
