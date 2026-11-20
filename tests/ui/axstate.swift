// usage: axstate <pid> — one JSON object with the SSHL window state for tests/ui/shortcuts.py: tabs (and which is selected),
// button labels, window titles, whether the window is minimized.
import AppKit
let pid = pid_t(CommandLine.arguments[1])!
func attr(_ e: AXUIElement, _ a: String) -> CFTypeRef? { var r: CFTypeRef?; AXUIElementCopyAttributeValue(e, a as CFString, &r); return r }
var tabs: [[String: Any]] = [], buttons: [String] = [], windows: [String] = [], minimized = false
func walk(_ e: AXUIElement, _ inTabs: Bool) {
    let role = attr(e, kAXRoleAttribute) as? String ?? ""
    let label = (attr(e, kAXDescriptionAttribute) as? String) ?? (attr(e, kAXTitleAttribute) as? String) ?? ""
    var nowInTabs = inTabs
    if role == "AXTabGroup" { nowInTabs = label == "已打开的会话" }
    if role == "AXRadioButton" && inTabs { tabs.append(["name": label, "selected": (attr(e, kAXValueAttribute) as? Int ?? 0) == 1]) }
    if role == "AXButton" { buttons.append(label) }
    if role == "AXWindow" { windows.append(label); if (attr(e, kAXMinimizedAttribute) as? Bool) == true { minimized = true } }
    for c in (attr(e, kAXChildrenAttribute) as? [AXUIElement]) ?? [] { walk(c, nowInTabs) }
}
walk(AXUIElementCreateApplication(pid), false)
let out: [String: Any] = ["tabs": tabs, "buttons": buttons, "windows": windows, "minimized": minimized]
print(String(data: try! JSONSerialization.data(withJSONObject: out), encoding: .utf8)!)
