#!/usr/bin/env bash
# 部署 SSHL.app 到 /Applications，并自动套上自定义图标。
#
# 为什么要套自定义图标：macOS 对「bundle app 图标」走 IconServices 模板+缓存渲染，
# 在铺满边角的深色 squircle 上会泛出一圈高亮/光环，且缓存顽固。把 icon.png 设为 Finder
# 自定义图标可绕开该渲染路径，直接用原图，效果与手动「显示简介→粘贴图标」一致。
#
# 用法: pnpm deploy   (需先 pnpm build 产出 bundle)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SRC="$ROOT/src-tauri/target/release/bundle/macos/SSHL.app"
DST="/Applications/SSHL.app"
ICON="$ROOT/icon.png"
LSREGISTER=/System/Library/Frameworks/CoreServices.framework/Versions/A/Frameworks/LaunchServices.framework/Versions/A/Support/lsregister

[ -d "$SRC" ] || { echo "✗ 未找到构建产物: $SRC（先跑 pnpm build）"; exit 1; }
[ -f "$ICON" ] || { echo "✗ 未找到图标: $ICON"; exit 1; }
command -v fileicon >/dev/null || { echo "✗ 缺少 fileicon（brew install fileicon）"; exit 1; }

# 退出正在运行的实例，避免占用
osascript -e 'quit app "SSHL"' 2>/dev/null || true
pkill -f "$DST" 2>/dev/null || true

# 先删旧 bundle 再拷新的（覆盖会让图标缓存按旧 inode 残留）
rm -rf "$DST"
ditto "$SRC" "$DST"

# 套自定义图标：先清残留再用规范 icon.png 设置
fileicon rm "$DST" 2>/dev/null || true
fileicon set "$DST" "$ICON"

# 刷新 LaunchServices + Dock/Finder
touch "$DST"
"$LSREGISTER" -f "$DST"
killall Dock Finder 2>/dev/null || true

echo "✓ 已部署并套用自定义图标: $DST"
