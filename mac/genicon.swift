// 生成 1024×1024 应用图标：暖琥珀圆角底 + 蚂蚁
// 用法：swift mac/genicon.swift   （在 mac/ 目录下运行，输出 build/icon-1024.png）
import AppKit

let px = 1024
guard let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: px, pixelsHigh: px,
                                 bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
                                 colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0) else {
    fatalError("无法创建位图")
}
rep.size = NSSize(width: px, height: px)

NSGraphicsContext.saveGraphicsState()
NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)

let size = CGFloat(px)
let inset: CGFloat = 36
let bg = NSBezierPath(roundedRect: NSRect(x: inset, y: inset, width: size - inset * 2, height: size - inset * 2),
                      xRadius: 230, yRadius: 230)
NSColor(srgbRed: 0.910, green: 0.392, blue: 0.110, alpha: 1).setFill()
bg.fill()

let para = NSMutableParagraphStyle()
para.alignment = .center
let text = NSAttributedString(string: "🐜", attributes: [
    .font: NSFont.systemFont(ofSize: 560, weight: .bold),
    .paragraphStyle: para,
])
let bounds = text.boundingRect(with: NSSize(width: size, height: size), options: [.usesLineFragmentOrigin])
text.draw(with: NSRect(x: 0, y: (size - bounds.height) / 2 - 12, width: size, height: bounds.height + 24),
          options: [.usesLineFragmentOrigin], context: nil)

NSGraphicsContext.restoreGraphicsState()

guard let png = rep.representation(using: .png, properties: [:]) else { fatalError("PNG 编码失败") }
try! FileManager.default.createDirectory(atPath: "build", withIntermediateDirectories: true)
try! png.write(to: URL(fileURLWithPath: "build/icon-1024.png"))
print("icon → build/icon-1024.png")
