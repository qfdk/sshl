# 给 SSHL.app 签名（scripts/bundle.sh、deploy.sh、release.sh 共用，source 进来用）。
# 里面的访达工具栏按钮（Contents/Helpers/Open in SSHL.app）要给访达发 Apple Events。它在 SSHL.app 里面，系统把它算成 SSHL 的：
# 查权限查的是 SSHL，开了 hardened runtime 时 SSHL 自己也得带 com.apple.security.automation.apple-events 授权
# （assets/entitlements.plist，签名器不认那里面的 XML 注释），不然直接拒绝、连询问框都不弹（tccd：「requires entitlement」）。
# 所以两层都带这个授权：先签里面，再签外面；不用 --deep，它会把里面重签一遍。
#
# 用法（先设 ROOT 为仓库根目录）: sign_app <SSHL.app> <签名身份，- 是 ad-hoc> [codesign 的其他参数，如 --options runtime --timestamp]
sign_app() {
  local app=$1 identity=$2
  shift 2
  local entitlements="${ROOT:?先设 ROOT（仓库根目录）}/assets/entitlements.plist"
  local helper="$app/Contents/Helpers/Open in SSHL.app"
  if [ -d "$helper" ]; then
    codesign --force "$@" --entitlements "$entitlements" -s "$identity" "$helper"
  fi
  codesign --force "$@" --entitlements "$entitlements" -s "$identity" "$app"
  codesign --verify --strict --deep "$app"
}
