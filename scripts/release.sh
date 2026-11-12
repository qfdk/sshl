#!/usr/bin/env bash
# 出发布包：Developer ID 签名 + 公证 + 装订票据。产物：target/release/SSHL-v<版本>-macos.zip。
#
# 证书从钥匙串自动查找，公证凭据用 notarytool 存在钥匙串里的配置，仓库里不写证书名和账号。
#
# 用法: NOTARY_PROFILE=<notarytool 钥匙串配置名> scripts/release.sh
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

: "${NOTARY_PROFILE:?先设置 NOTARY_PROFILE（xcrun notarytool store-credentials 存的配置名）}"
SIGN_ID=$(security find-identity -v -p codesigning | awk -F'"' '/Developer ID Application/{print $2; exit}')
[ -n "$SIGN_ID" ] || { echo "✗ 钥匙串里没有 Developer ID Application 证书"; exit 1; }

scripts/bundle.sh

APP="$ROOT/target/release/bundle/SSHL.app"
VERSION=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$APP/Contents/Info.plist")
ZIP="$ROOT/target/release/SSHL-v${VERSION}-macos.zip"

# 公证要求 Hardened Runtime 和带时间戳的签名
codesign --force --options runtime --timestamp -s "$SIGN_ID" "$APP"
codesign --verify --strict "$APP"

rm -f "$ZIP"
ditto -c -k --keepParent "$APP" "$ZIP"
xcrun notarytool submit "$ZIP" --keychain-profile "$NOTARY_PROFILE" --wait

# 票据装订进 app 后重新打包：没联网的机器首次打开也能通过检查
xcrun stapler staple "$APP"
rm -f "$ZIP"
ditto -c -k --keepParent "$APP" "$ZIP"
spctl -a -vv "$APP"

echo "✓ ${ZIP}（$(du -h "$ZIP" | cut -f1)）"
