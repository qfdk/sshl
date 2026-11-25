// usage: keypost <pid> <keycode> [mods: none | cmd | shift | cmd+shift | ctrl | ctrl+cmd | fn] — posts a real key down/up to the process
import AppKit
let pid = pid_t(CommandLine.arguments[1])!, code = CGKeyCode(CommandLine.arguments[2])!
let mods = CommandLine.arguments.count > 3 ? CommandLine.arguments[3] : "none"
var flags: CGEventFlags = []
if mods.contains("cmd") { flags.insert(.maskCommand) }
if mods.contains("shift") { flags.insert(.maskShift) }
if mods.contains("ctrl") { flags.insert(.maskControl) }
if mods.contains("fn") { flags.insert(.maskSecondaryFn) }
let src = CGEventSource(stateID: .privateState)
if !flags.isEmpty, let e = CGEvent(source: src) { e.type = .flagsChanged; e.flags = flags; e.postToPid(pid) }
for down in [true, false] { if let e = CGEvent(keyboardEventSource: src, virtualKey: code, keyDown: down) { e.flags = flags; e.postToPid(pid) }; usleep(20_000) }
if !flags.isEmpty, let e = CGEvent(source: src) { e.type = .flagsChanged; e.flags = []; e.postToPid(pid) }
