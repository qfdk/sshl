// usage: axdump <pid> [filter] — prints the accessibility tree of a process: role | label | centre in window coordinates
// (what the test hook's `click X Y` / `press X Y` take).
import AppKit
let pid = pid_t(CommandLine.arguments[1])!
let filter = CommandLine.arguments.count > 2 ? CommandLine.arguments[2] : ""
func attr(_ e: AXUIElement, _ a: String) -> CFTypeRef? { var r: CFTypeRef?; AXUIElementCopyAttributeValue(e, a as CFString, &r); return r }
var winOrigin = CGPoint.zero
func walk(_ e: AXUIElement, _ depth: Int) {
    let role = attr(e, kAXRoleAttribute) as? String ?? ""
    let label = (attr(e, kAXDescriptionAttribute) as? String) ?? (attr(e, kAXTitleAttribute) as? String) ?? ""
    var pos = CGPoint.zero, size = CGSize.zero
    if let p = attr(e, kAXPositionAttribute) { AXValueGetValue(p as! AXValue, .cgPoint, &pos) }
    if let s = attr(e, kAXSizeAttribute) { AXValueGetValue(s as! AXValue, .cgSize, &size) }
    if role == "AXWindow" { winOrigin = pos }
    if filter.isEmpty || label.contains(filter) || role.contains(filter) {
        print(String(repeating: " ", count: depth), role, "|", label, "| win-xy", Int(pos.x - winOrigin.x + size.width/2), Int(pos.y - winOrigin.y + size.height/2))
    }
    for c in (attr(e, kAXChildrenAttribute) as? [AXUIElement]) ?? [] { walk(c, depth + 1) }
}
walk(AXUIElementCreateApplication(pid), 0)
