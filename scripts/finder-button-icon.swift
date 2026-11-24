// 访达工具栏按钮的图标：灰色线框里一个「>_」，同 Lucide 的 terminal 和 OpenInTerminal-Lite 的风格（访达工具栏上别的图标也是灰色线条）。
// 用户 2026-11-24 一路试过：SSHL 的彩色图标放工具栏上「颜色不好看」；框里写字体的「$」不如线条好看；线条画的「$ |」「# |」
// 都挤在框的左边、右半边空着（「$ 过于占地方」）；「> |」像播放器的「跳到末尾」。「>_」的尖角在上、下划线往右下铺开，正好撑满横向的框；
// 和 OpenInTerminal 撞了也没关系（用户：「撞了应该问题也不大」）。
// 重新生成：swift scripts/finder-button-icon.swift /tmp/x.iconset && iconutil -c icns /tmp/x.iconset -o assets/finder-button/icon.icns
import AppKit

let gray = NSColor(srgbRed: 0x66 / 255.0, green: 0x66 / 255.0, blue: 0x66 / 255.0, alpha: 1)
let stroke: CGFloat = 10

func render(_ pixels: Int) -> NSBitmapImageRep {
    let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: pixels, pixelsHigh: pixels, bitsPerSample: 8, samplesPerPixel: 4,
                               hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
    let s = CGFloat(pixels) / 256
    // 按 256 的画布、左上角为原点来写
    func pt(_ x: CGFloat, _ y: CGFloat) -> NSPoint { NSPoint(x: x * s, y: (256 - y) * s) }
    gray.setStroke()
    let frame = NSBezierPath(roundedRect: NSRect(x: 35.5 * s, y: (256 - 53.5 - 147) * s, width: 185 * s, height: 147 * s), xRadius: 22 * s, yRadius: 22 * s)
    frame.lineWidth = stroke * s
    frame.stroke()

    // 「>」和「_」：和外框同粗的圆头线条
    let glyph = NSBezierPath()
    glyph.move(to: pt(76, 100))
    glyph.line(to: pt(100, 122))
    glyph.line(to: pt(76, 144))
    glyph.move(to: pt(112, 160))
    glyph.line(to: pt(150, 160))
    glyph.lineWidth = stroke * s
    glyph.lineCapStyle = .round
    glyph.lineJoinStyle = .round
    glyph.stroke()
    NSGraphicsContext.restoreGraphicsState()
    return rep
}

let out = URL(fileURLWithPath: CommandLine.arguments[1])
try FileManager.default.createDirectory(at: out, withIntermediateDirectories: true)
for (name, pixels) in [("16x16", 16), ("16x16@2x", 32), ("32x32", 32), ("32x32@2x", 64), ("128x128", 128), ("128x128@2x", 256),
                       ("256x256", 256), ("256x256@2x", 512), ("512x512", 512), ("512x512@2x", 1024)] {
    try render(pixels).representation(using: .png, properties: [:])!.write(to: out.appendingPathComponent("icon_\(name).png"))
}
