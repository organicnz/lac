import AppKit
import CoreGraphics

// Master 1024x1024 Apple Executive Liquid Glass Loop App Icon
let size: CGFloat = 1024.0
let img = NSImage(size: NSSize(width: size, height: size))
img.lockFocusFlipped(false)
guard let ctx = NSGraphicsContext.current?.cgContext else {
    fputs("Error: No graphics context\n", stderr)
    exit(1)
}

let colorSpace = CGColorSpaceCreateDeviceRGB()

// 1. Full-bleed Deep Space Obsidian Canvas
let bgRect = CGRect(x: 0, y: 0, width: size, height: size)
let darkTop = CGColor(colorSpace: colorSpace, components: [0.04, 0.05, 0.09, 1.0])!
let darkBottom = CGColor(colorSpace: colorSpace, components: [0.01, 0.01, 0.03, 1.0])!
let bgGrad = CGGradient(colorsSpace: colorSpace, colors: [darkTop, darkBottom] as CFArray, locations: [0.0, 1.0])!
ctx.drawLinearGradient(bgGrad, start: CGPoint(x: size/2, y: size), end: CGPoint(x: size/2, y: 0), options: [])

// 2. Ambient Radial Caustic / Internal Optical Glow
let centerGlow = CGColor(colorSpace: colorSpace, components: [0.12, 0.28, 0.72, 0.35])!
let centerClear = CGColor(colorSpace: colorSpace, components: [0.0, 0.0, 0.0, 0.0])!
let radialGrad = CGGradient(colorsSpace: colorSpace, colors: [centerGlow, centerClear] as CFArray, locations: [0.0, 1.0])!
ctx.drawRadialGradient(radialGrad, startCenter: CGPoint(x: size/2, y: size/2), startRadius: 0, endCenter: CGPoint(x: size/2, y: size/2), endRadius: size * 0.45, options: [])

// 3. Mathematical Lemniscate of Bernoulli Path
func makeLemniscatePath(width: CGFloat, height: CGFloat, center: CGPoint) -> CGPath {
    let path = CGMutablePath()
    let a = (width / 2.0) * 0.88
    let yScale = (height / 2.0) * 1.55
    let steps = 240
    
    for i in 0...steps {
        let t = (Double(i) / Double(steps)) * 2.0 * .pi
        let sinT = sin(t)
        let cosT = cos(t)
        let denom = 1.0 + sinT * sinT
        let x = center.x + CGFloat((a * cosT) / denom)
        let y = center.y + CGFloat((Double(yScale) * sinT * cosT) / denom)
        
        if i == 0 {
            path.move(to: CGPoint(x: x, y: y))
        } else {
            path.addLine(to: CGPoint(x: x, y: y))
        }
    }
    path.closeSubpath()
    return path
}

let loopCenter = CGPoint(x: size/2, y: size/2)
let loopPath = makeLemniscatePath(width: size * 0.68, height: size * 0.44, center: loopCenter)

// 4. Multi-Layer Glow (Neon Light-Pipe)
ctx.saveGState()
ctx.setShadow(offset: .zero, blur: size * 0.09, color: CGColor(colorSpace: colorSpace, components: [0.15, 0.55, 1.0, 0.85])!)
ctx.addPath(loopPath)
ctx.setLineWidth(size * 0.09)
ctx.setLineCap(.round)
ctx.setLineJoin(.round)
ctx.setStrokeColor(CGColor(colorSpace: colorSpace, components: [0.20, 0.60, 1.0, 0.60])!)
ctx.strokePath()
ctx.restoreGState()

ctx.saveGState()
ctx.setShadow(offset: .zero, blur: size * 0.05, color: CGColor(colorSpace: colorSpace, components: [0.75, 0.25, 0.95, 0.80])!)
ctx.addPath(loopPath)
ctx.setLineWidth(size * 0.06)
ctx.setLineCap(.round)
ctx.setLineJoin(.round)
ctx.setStrokeColor(CGColor(colorSpace: colorSpace, components: [0.80, 0.35, 1.0, 0.70])!)
ctx.strokePath()
ctx.restoreGState()

// 5. Optical Glass Ribbon Body
ctx.saveGState()
ctx.addPath(loopPath)
ctx.setLineWidth(size * 0.055)
ctx.setLineCap(.round)
ctx.setLineJoin(.round)
ctx.replacePathWithStrokedPath()
ctx.clip()

// Gradient fill inside ribbon
let cyan = CGColor(colorSpace: colorSpace, components: [0.10, 0.85, 1.0, 0.95])!
let cobalt = CGColor(colorSpace: colorSpace, components: [0.12, 0.45, 0.98, 0.95])!
let violet = CGColor(colorSpace: colorSpace, components: [0.72, 0.28, 0.98, 0.95])!
let magenta = CGColor(colorSpace: colorSpace, components: [0.98, 0.25, 0.60, 0.95])!
let ribbonGrad = CGGradient(colorsSpace: colorSpace, colors: [cyan, cobalt, violet, magenta] as CFArray, locations: [0.0, 0.35, 0.70, 1.0])!
ctx.drawLinearGradient(ribbonGrad, start: CGPoint(x: size * 0.2, y: size * 0.7), end: CGPoint(x: size * 0.8, y: size * 0.3), options: [])
ctx.restoreGState()

// 6. Specular Highlight (Liquid Glass Sheen)
ctx.saveGState()
ctx.addPath(loopPath)
ctx.setLineWidth(size * 0.015)
ctx.setLineCap(.round)
ctx.setLineJoin(.round)
ctx.setStrokeColor(CGColor(colorSpace: colorSpace, components: [1.0, 1.0, 1.0, 0.85])!)
ctx.strokePath()
ctx.restoreGState()

// 7. Center Crossing Laser Sparkle
ctx.saveGState()
ctx.setShadow(offset: .zero, blur: size * 0.03, color: CGColor(colorSpace: colorSpace, components: [1.0, 1.0, 1.0, 0.95])!)
let sparkleRect = CGRect(x: loopCenter.x - size * 0.018, y: loopCenter.y - size * 0.018, width: size * 0.036, height: size * 0.036)
ctx.setFillColor(CGColor(colorSpace: colorSpace, components: [1.0, 1.0, 1.0, 1.0])!)
ctx.fillEllipse(in: sparkleRect)
ctx.restoreGState()

img.unlockFocus()

guard let tiff = img.tiffRepresentation,
      let rep = NSBitmapImageRep(data: tiff),
      let png = rep.representation(using: .png, properties: [:]) else {
    fputs("Error: PNG encode failed\n", stderr)
    exit(1)
}

let outPath = CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "icon_1024.png"
try png.write(to: URL(fileURLWithPath: outPath))
print("Successfully generated master icon at \(outPath)")
