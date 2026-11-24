// 访达工具栏按钮「在 SSHL 中打开」的图标：同 OpenInTerminal-Lite 的灰色线框风格（访达工具栏上别的图标也是灰色线条），
// 框里是「$ |」：提示符加 SSHL 终端默认的竖线光标（用户 2026-11-24：「不要用方块」「$ |」）。用户 2026-11-24：用 SSHL 的彩色应用图标放在工具栏上「颜色并不是特别好看」。
// 重新生成：swift scripts/finder-button-icon.swift /tmp/x.iconset && iconutil -c icns /tmp/x.iconset -o assets/finder-button/icon.icns
import AppKit

let gray = NSColor(srgbRed: 0x66 / 255.0, green: 0x66 / 255.0, blue: 0x66 / 255.0, alpha: 1)

func render(_ pixels: Int) -> NSBitmapImageRep {
    let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: pixels, pixelsHigh: pixels, bitsPerSample: 8, samplesPerPixel: 4,
                               hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
    let s = CGFloat(pixels) / 256
    // 按 256 的画布、左上角为原点来写，换成 AppKit 的左下角原点
    func rect(_ x: CGFloat, _ y: CGFloat, _ w: CGFloat, _ h: CGFloat) -> NSRect { NSRect(x: x * s, y: (256 - y - h) * s, width: w * s, height: h * s) }
    gray.setStroke()
    gray.setFill()
    let frame = NSBezierPath(roundedRect: rect(35.5, 53.5, 185, 147), xRadius: 22 * s, yRadius: 22 * s)
    frame.lineWidth = 10 * s
    frame.stroke()
    // 「$」：等宽半粗，和框的线差不多粗（几版对比过：粗体太重，竖线离远了显得散）
    let font = NSFont.monospacedSystemFont(ofSize: 70 * s, weight: .semibold)
    let dollar = NSAttributedString(string: "$", attributes: [.font: font, .foregroundColor: gray])
    let size = dollar.size()
    dollar.draw(at: NSPoint(x: 70 * s, y: (256 - 127) * s - size.height / 2))
    // 竖线光标：和框一样粗，紧跟在「$」后面靠左（同终端里提示符后的光标；用户：「不要太居中，要靠左」）
    NSBezierPath(roundedRect: rect(124, 100, 9, 54), xRadius: 4.5 * s, yRadius: 4.5 * s).fill()
    NSGraphicsContext.restoreGraphicsState()
    return rep
}

let out = URL(fileURLWithPath: CommandLine.arguments[1])
try FileManager.default.createDirectory(at: out, withIntermediateDirectories: true)
for (name, pixels) in [("16x16", 16), ("16x16@2x", 32), ("32x32", 32), ("32x32@2x", 64), ("128x128", 128), ("128x128@2x", 256),
                       ("256x256", 256), ("256x256@2x", 512), ("512x512", 512), ("512x512@2x", 1024)] {
    try render(pixels).representation(using: .png, properties: [:])!.write(to: out.appendingPathComponent("icon_\(name).png"))
}
