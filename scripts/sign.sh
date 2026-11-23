# 给 SSHL.app 签名（scripts/bundle.sh、deploy.sh、release.sh 共用，source 进来用）。
# 先签里面的访达工具栏按钮（Contents/Helpers/Open in SSHL.app）：它要给访达发 Apple Events，开了 hardened runtime 时
# 得带 com.apple.security.automation.apple-events 授权（assets/finder-button/entitlements.plist，签名器不认那里面的 XML 注释）。再签外面：不用 --deep，它会把里面重签一遍、把授权冲掉。
#
# 用法（先设 ROOT 为仓库根目录）: sign_app <SSHL.app> <签名身份，- 是 ad-hoc> [codesign 的其他参数，如 --options runtime --timestamp]
sign_app() {
  local app=$1 identity=$2
  shift 2
  local helper="$app/Contents/Helpers/Open in SSHL.app"
  if [ -d "$helper" ]; then
    codesign --force "$@" --entitlements "${ROOT:?先设 ROOT（仓库根目录）}/assets/finder-button/entitlements.plist" -s "$identity" "$helper"
  fi
  codesign --force "$@" -s "$identity" "$app"
  codesign --verify --strict --deep "$app"
}
