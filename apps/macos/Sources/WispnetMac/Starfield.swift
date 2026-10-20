import SwiftUI

/// The Linux desktop's starfield, drawn natively at the same 30 fps cadence.
struct Starfield: View {
    static let jumpDuration = 0.9
    let online: Bool
    let warpStartedAt: TimeInterval?
    let active: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private struct Star {
        let angle: Double
        let depth: Double
        let speed: Double
    }
    private static let referenceOrigin = Date.timeIntervalSinceReferenceDate - ProcessInfo.processInfo.systemUptime
    private static let stars: [Star] = {
        var seed: UInt64 = 17
        func random() -> Double {
            seed &+= 0x9e3779b97f4a7c15
            var value = seed
            value = (value ^ (value >> 30)) &* 0xbf58476d1ce4e5b9
            value = (value ^ (value >> 27)) &* 0x94d049bb133111eb
            return Double((value ^ (value >> 31)) >> 11) / 9_007_199_254_740_992
        }
        return (0..<96).map { _ in
            Star(angle: random() * .pi * 2, depth: random(), speed: 0.6 + random() * 0.8)
        }
    }()

    var body: some View {
        TimelineView(.animation(minimumInterval: 1.0 / 30, paused: !active || reduceMotion)) { timeline in
            let now = reduceMotion ? 0 : timeline.date.timeIntervalSinceReferenceDate - Self.referenceOrigin
            Canvas(opaque: true, rendersAsynchronously: true) { context, size in
                draw(in: &context, size: size, now: now)
            }
        }
        .allowsHitTesting(false)
        .accessibilityHidden(true)
    }

    private func draw(in context: inout GraphicsContext, size: CGSize, now: Double) {
        let width = size.width, height = size.height
        let center = CGPoint(x: width / 2, y: height / 2)
        let scale = hypot(center.x, center.y)
        let rectangle = Path(CGRect(origin: .zero, size: size))
        let elapsed = warpStartedAt.map { now - $0 } ?? 999
        let warp = !reduceMotion && elapsed >= 0 && elapsed < Self.jumpDuration
        let progress = warp ? min(1, max(0, elapsed / Self.jumpDuration)) : 0
        let lockOn = warp && progress < 0.18 ? sin((progress / 0.18) * .pi) : 0
        let warpPhase = warp ? max(0, (progress - 0.14) / 0.86) : 0
        let surge = warp ? sin(.pi * pow(warpPhase, 0.75)) : 0
        let palette = online ? [0x12244a, 0x0c1834, 0x081022] : [0x0e1930, 0x0a1224, 0x070c18]
        context.fill(rectangle, with: .linearGradient(Gradient(stops: [
            .init(color: Color(starHex: palette[0]), location: 0),
            .init(color: Color(starHex: palette[1]), location: 0.55),
            .init(color: Color(starHex: palette[2]), location: 1),
        ]), startPoint: .zero, endPoint: CGPoint(x: width, y: height)))

        let pulse = 0.82 + 0.18 * sin(now * 1.6)
        context.fill(rectangle, with: .radialGradient(Gradient(stops: [
            .init(color: Color(starHex: 0x3876e0).opacity((online ? 110 : 68) / 255 * pulse), location: 0),
            .init(color: Color(starHex: 0x183470).opacity(45 / 255 * pulse), location: 0.55),
            .init(color: .clear, location: 1),
        ]), center: center, startRadius: 0, endRadius: max(1, width * 0.45)))

        if warp && surge > 0 {
            context.fill(rectangle, with: .radialGradient(Gradient(stops: [
                .init(color: Color(starHex: 0xe4fbff).opacity(180 / 255 * surge), location: 0),
                .init(color: Color(starHex: 0x65ceff).opacity(120 / 255 * surge), location: 0.2),
                .init(color: .clear, location: 1),
            ]), center: center, startRadius: 0, endRadius: max(1, width * 0.45)))
        }
        if warp && progress > 0.76 {
            let wave = (progress - 0.76) / 0.24
            let radius = wave * scale * 0.65
            let ring = Path(ellipseIn: CGRect(x: center.x - radius, y: center.y - radius,
                                              width: radius * 2, height: radius * 2))
            context.stroke(ring, with: .color(Color(starHex: 0x65ceff).opacity((1 - wave) * 165 / 255)),
                           lineWidth: max(1, 2.8 - wave * 2))
        }

        let cruise = (now * (online ? 1.8 : 1) / 24).truncatingRemainder(dividingBy: 1)
        for (index, star) in Self.stars.enumerated() {
            let depth = (star.depth + cruise * star.speed + warpPhase * 0.75).truncatingRemainder(dividingBy: 1)
            let radius = max(4, (0.03 + depth * depth - lockOn * 0.02) * scale)
            let ux = cos(star.angle), uy = sin(star.angle)
            let twinkle = 0.7 + 0.3 * sin(now * 4 * (0.5 + star.speed) + Double(index))
            let tail = warp ? surge * surge * (0.025 + depth * 0.36) * scale
                            : depth * depth * (online ? 14 : 5.5)
            let hex = index % 15 == 0 ? 0xffdfa8 : index % 9 == 0 ? 0x7da2ff : index % 6 == 0 ? 0x65ceff : 0xf0f8ff
            let alpha = min(255, (warp ? 125 + star.speed * 115 : (75 + depth * 155) * twinkle)
                            * (warp ? min(1, 0.35 + surge * 3) : 1)) / 255
            let color = Color(starHex: hex).opacity(alpha)
            let lineWidth = warp ? 1.2 + star.speed * 1.5 : 1 + depth * 1.8
            let point = CGPoint(x: center.x + ux * radius, y: center.y + uy * radius)
            if tail < 1.5 {
                context.fill(Path(ellipseIn: CGRect(x: point.x - lineWidth / 2, y: point.y - lineWidth / 2,
                                                    width: lineWidth, height: lineWidth)), with: .color(color))
            } else {
                var streak = Path()
                streak.move(to: point)
                streak.addLine(to: CGPoint(x: center.x + ux * (radius + tail), y: center.y + uy * (radius + tail)))
                context.stroke(streak, with: .color(color), style: StrokeStyle(lineWidth: lineWidth, lineCap: .round))
            }
        }
    }
}

private extension Color {
    init(starHex hex: Int) {
        self.init(red: Double((hex >> 16) & 255) / 255,
                  green: Double((hex >> 8) & 255) / 255,
                  blue: Double(hex & 255) / 255)
    }
}
