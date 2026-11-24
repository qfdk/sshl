// 访达工具栏按钮的图标：同 OpenInTerminal-Lite 的灰色线框风格（访达工具栏上别的图标也是灰色线条），框里是「$ |」：
// 提示符加 SSHL 终端默认的竖线光标。用户 2026-11-24：SSHL 的彩色图标放在工具栏上「颜色并不是特别好看」，「不要用方块」，
// 竖线「不要太居中，要靠左」；用字体里的「$」时「不如 OpenInTerminal 的好看」——它的「>_」是和外框同粗、圆头的线条，
// 所以「$」也照 Lucide 的 dollar-sign 用同粗的圆头线条画（几版并排比过，这一版的「$」够大、中间不糊）。
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

    // 「$」：Lucide dollar-sign 的形状（24 格坐标），放大 3.6 倍，竖笔只伸出 S 上下各 1.6 格
    let scale: CGFloat = 3.6, ox: CGFloat = 66, oy: CGFloat = 102
    func lucide(_ x: CGFloat, _ y: CGFloat) -> NSPoint { pt(ox + (x - 6) * scale, oy + (y - 5) * scale) }
    let dollar = NSBezierPath()
    dollar.move(to: lucide(12, 5 - 1.6))
    dollar.line(to: lucide(12, 19 + 1.6))
    dollar.move(to: lucide(17, 5))
    dollar.line(to: lucide(9.5, 5))
    for i in 1...48 {
        let t = -CGFloat.pi / 2 - CGFloat.pi * CGFloat(i) / 48
        dollar.line(to: lucide(9.5 + 3.5 * cos(t), 8.5 + 3.5 * sin(t)))
    }
    dollar.line(to: lucide(14.5, 12))
    for i in 1...48 {
        let t = -CGFloat.pi / 2 + CGFloat.pi * CGFloat(i) / 48
        dollar.line(to: lucide(14.5 + 3.5 * cos(t), 15.5 + 3.5 * sin(t)))
    }
    dollar.line(to: lucide(6, 19))
    // 竖线光标：一整行高，跟在「$」后面隔一小段
    dollar.move(to: pt(136, 96))
    dollar.line(to: pt(136, 158))
    dollar.lineWidth = stroke * s
    dollar.lineCapStyle = .round
    dollar.lineJoinStyle = .round
    dollar.stroke()
    NSGraphicsContext.restoreGraphicsState()
    return rep
}

let out = URL(fileURLWithPath: CommandLine.arguments[1])
try FileManager.default.createDirectory(at: out, withIntermediateDirectories: true)
for (name, pixels) in [("16x16", 16), ("16x16@2x", 32), ("32x32", 32), ("32x32@2x", 64), ("128x128", 128), ("128x128@2x", 256),
                       ("256x256", 256), ("256x256@2x", 512), ("512x512", 512), ("512x512@2x", 1024)] {
    try render(pixels).representation(using: .png, properties: [:])!.write(to: out.appendingPathComponent("icon_\(name).png"))
}
