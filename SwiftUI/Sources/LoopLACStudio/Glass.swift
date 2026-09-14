import AppKit
import SwiftUI

// MARK: - Liquid Glass Design System
//
// Native Apple Silicon macOS design system implementing the Liquid Glass
// aesthetic. Strategy (Apple HIG, macOS Tahoe 26+):
//   - On macOS 26+: prefer the system `.glassEffect` refraction engine
//     (real-time specular + lensing, automatic reduce-transparency handling).
//   - On macOS 13–15: fall back to the hand-tuned stack below
//     (ultraThinMaterial + specular sheen/border + layered shadows).
//   - Motion/contrast fallbacks are always respected via
//     accessibilityReduceMotion / reduceTransparency / increased contrast.
// Fully compatible with macOS 13+.

public enum LiquidGlass {
    /// 120Hz fluid interpolating spring for Apple Silicon ProMotion displays.
    public static let spring = Animation.interpolatingSpring(stiffness: 320, damping: 24)
    public static let responsiveSpring = Animation.interpolatingSpring(stiffness: 400, damping: 28)
    public static let gentleSpring = Animation.interpolatingSpring(stiffness: 220, damping: 20)

    /// Specular hairline border simulating glass light refraction.
    public static func specularBorder(cornerRadius: CGFloat = 12, isHovered: Bool = false) -> some View {
        RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
            .strokeBorder(
                LinearGradient(
                    stops: [
                        .init(color: .white.opacity(isHovered ? 0.45 : 0.28), location: 0.0),
                        .init(color: .white.opacity(isHovered ? 0.20 : 0.10), location: 0.4),
                        .init(color: .clear, location: 0.6),
                        .init(color: .white.opacity(isHovered ? 0.15 : 0.06), location: 1.0)
                    ],
                    startPoint: .topLeading,
                    endPoint: .bottomTrailing
                ),
                lineWidth: 1
            )
    }

    /// Top specular sheen for high refractive realism.
    public static func specularSheen(cornerRadius: CGFloat = 12) -> some View {
        RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
            .fill(
                LinearGradient(
                    colors: [
                        Color.white.opacity(0.10),
                        Color.white.opacity(0.02),
                        Color.clear
                    ],
                    startPoint: .top,
                    endPoint: .center
                )
            )
            .allowsHitTesting(false)
    }

    /// Trigger tactile feedback on macOS trackpads.
    public static func haptic(_ type: NSHapticFeedbackManager.FeedbackPattern = .alignment) {
        NSHapticFeedbackManager.defaultPerformer.perform(type, performanceTime: .default)
    }
}

// MARK: - Native AppKit Visual Effect View (Dynamic Desktop Blur)

public struct VisualEffectView: NSViewRepresentable {
    public var material: NSVisualEffectView.Material
    public var blendingMode: NSVisualEffectView.BlendingMode
    public var state: NSVisualEffectView.State

    public init(
        material: NSVisualEffectView.Material = .underWindowBackground,
        blendingMode: NSVisualEffectView.BlendingMode = .behindWindow,
        state: NSVisualEffectView.State = .active
    ) {
        self.material = material
        self.blendingMode = blendingMode
        self.state = state
    }

    public func makeNSView(context: Context) -> NSVisualEffectView {
        let view = NSVisualEffectView()
        view.material = material
        view.blendingMode = blendingMode
        view.state = state
        return view
    }

    public func updateNSView(_ nsView: NSVisualEffectView, context: Context) {
        nsView.material = material
        nsView.blendingMode = blendingMode
        nsView.state = state
    }
}

// MARK: - Window Glass Accessor

public struct WindowGlassAccessor: NSViewRepresentable {
    public init() {}

    public func makeNSView(context: Context) -> NSView {
        let view = NSView()
        DispatchQueue.main.async {
            if let window = view.window {
                AppDelegate.mainWindow = window
                window.isOpaque = false
                window.backgroundColor = .clear
                window.titlebarAppearsTransparent = true
                window.titleVisibility = .hidden
                window.isMovableByWindowBackground = true
                window.styleMask.insert(.fullSizeContentView)
                window.makeKeyAndOrderFront(nil)
                window.orderFrontRegardless()
            }
        }
        return view
    }

    public func updateNSView(_ nsView: NSView, context: Context) {}
}

// MARK: - Liquid Glass Badge

public struct LiquidGlassBadge: View {
    public var title: String
    public var icon: String?
    public var color: Color
    public var isPulsing: Bool

    public init(title: String, icon: String? = nil, color: Color = .green, isPulsing: Bool = false) {
        self.title = title
        self.icon = icon
        self.color = color
        self.isPulsing = isPulsing
    }

    @State private var pulse = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private var pulsing: Bool { isPulsing && !reduceMotion }

    public var body: some View {
        HStack(spacing: 5) {
            Circle()
                .fill(color)
                .frame(width: 7, height: 7)
                .scaleEffect(pulse && pulsing ? 1.2 : 1.0)
                .opacity(pulse && pulsing ? 0.7 : 1.0)
                .animation(pulsing ? Animation.easeInOut(duration: 1.2).repeatForever(autoreverses: true) : .default, value: pulse)
            if let icon = icon {
                Image(systemName: icon)
                    .font(.system(size: 10, weight: .medium))
                    .foregroundColor(color)
            }
            Text(title)
                .font(.system(size: 11, weight: .medium))
                .foregroundColor(.primary)
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 4)
        .background(
            Capsule()
                .fill(.ultraThinMaterial)
                .overlay(
                    Capsule()
                        .fill(color.opacity(0.12))
                )
                .overlay(
                    Capsule()
                        .strokeBorder(color.opacity(0.35), lineWidth: 1)
                )
        )
        .onAppear {
            if pulsing { pulse = true }
        }
    }
}

// MARK: - Liquid Glass Card Modifier

public struct LiquidGlassCardModifier: ViewModifier {
    var cornerRadius: CGFloat
    var hoverable: Bool
    var tintColor: Color?
    @State private var isHovered = false
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency
    @Environment(\.colorSchemeContrast) private var contrast
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private var increaseContrast: Bool { contrast == .increased }

    /// Opaque fallback: translucency and perpetual motion off.
    private var reduced: Bool { reduceTransparency || increaseContrast }

    public func body(content: Content) -> some View {
        content
            .background(
                ZStack {
                    if reduced {
                        RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
                            .fill(Color(nsColor: .windowBackgroundColor))
                        RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
                            .strokeBorder(
                                Color.primary.opacity(increaseContrast ? 0.35 : 0.18),
                                lineWidth: 1
                            )
                    } else {
                        // Base material backdrop blur
                        RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
                            .fill(.ultraThinMaterial)

                        // Optional subtle color tint
                        if let tint = tintColor {
                            RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
                                .fill(tint.opacity(0.07))
                        }

                        // Top specular light sheen
                        LiquidGlass.specularSheen(cornerRadius: cornerRadius)

                        // Specular refraction border
                        LiquidGlass.specularBorder(cornerRadius: cornerRadius, isHovered: hoverable && isHovered)
                    }
                }
            )
            // Single static shadow when reduced; layered optical shadow otherwise.
            .shadow(
                color: Color.black.opacity(reduced ? 0.18 : (isHovered && hoverable ? 0.18 : 0.10)),
                radius: reduced ? 8 : (isHovered && hoverable ? 16 : 10),
                x: 0,
                y: reduced ? 3 : (isHovered && hoverable ? 8 : 4)
            )
            .shadow(
                color: Color.black.opacity(reduced ? 0 : 0.04),
                radius: 2,
                x: 0,
                y: 1
            )
            .scaleEffect(!reduced && !reduceMotion && isHovered && hoverable ? 1.008 : 1.0)
            .animation(reduceMotion ? .none : LiquidGlass.spring, value: isHovered)
            .onHover { hovering in
                if hoverable {
                    isHovered = hovering
                }
            }
    }
}

// MARK: - Liquid Glass Button Style

public struct LiquidGlassButtonStyle: ButtonStyle {
    var cornerRadius: CGFloat = 8
    var isProminent: Bool = false
    @State private var isHovered = false

    public func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .padding(.horizontal, 12)
            .padding(.vertical, 6)
            .background(
                ZStack {
                    if isProminent {
                        LinearGradient(
                            colors: [
                                Color.accentColor,
                                Color.accentColor.opacity(0.82)
                            ],
                            startPoint: .top,
                            endPoint: .bottom
                        )
                    } else {
                        RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
                            .fill(.ultraThinMaterial)
                        if isHovered {
                            RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
                                .fill(Color.white.opacity(0.08))
                        }
                    }

                    // Specular border
                    RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
                        .strokeBorder(
                            LinearGradient(
                                colors: [
                                    Color.white.opacity(isProminent ? 0.45 : (isHovered ? 0.35 : 0.18)),
                                    Color.white.opacity(isProminent ? 0.15 : (isHovered ? 0.12 : 0.05))
                                ],
                                startPoint: .topLeading,
                                endPoint: .bottomTrailing
                            ),
                            lineWidth: 1
                        )
                }
                .clipShape(RoundedRectangle(cornerRadius: cornerRadius, style: .continuous))
            )
            .foregroundColor(isProminent ? .white : .primary)
            .shadow(
                color: isProminent ? Color.accentColor.opacity(0.25) : Color.black.opacity(0.06),
                radius: isHovered ? 8 : 4,
                x: 0,
                y: isHovered ? 4 : 2
            )
            .scaleEffect(configuration.isPressed ? 0.96 : (isHovered ? 1.02 : 1.0))
            .animation(LiquidGlass.responsiveSpring, value: configuration.isPressed)
            .animation(LiquidGlass.spring, value: isHovered)
            .onHover { hovering in
                isHovered = hovering
            }
            .onChange(of: configuration.isPressed) { pressed in
                if pressed {
                    LiquidGlass.haptic(.alignment)
                }
            }
    }
}

// MARK: - View Extensions

extension View {
    /// Applies a translucent Liquid Glass card effect with specular border and layered shadow.
    ///
    /// Best practice: on macOS Tahoe (26+) this delegates to the system
    /// `.glassEffect` refraction engine; on older macOS it uses the
    /// hand-tuned ultraThinMaterial fallback with a11y-safe opaque mode.
    @ViewBuilder
    public func liquidGlassCard(cornerRadius: CGFloat = 12, hoverable: Bool = true, tint: Color? = nil) -> some View {
        if #available(macOS 26.0, *) {
            if tint == nil {
                self.glassEffect(.regular, in: RoundedRectangle(cornerRadius: cornerRadius, style: .continuous))
            } else {
                // Tinted cards keep the manual stack so brand color survives;
                // untinted cards get the true system refraction engine.
                self.modifier(LiquidGlassCardModifier(cornerRadius: cornerRadius, hoverable: hoverable, tintColor: tint))
            }
        } else {
            self.modifier(LiquidGlassCardModifier(cornerRadius: cornerRadius, hoverable: hoverable, tintColor: tint))
        }
    }

    /// Liquid Glass styling for standard buttons.
    public func lacGlass() -> some View {
        self.buttonStyle(LiquidGlassButtonStyle(cornerRadius: 8, isProminent: false))
    }

    /// Liquid Glass styling for prominent / primary buttons.
    public func lacGlassProminent() -> some View {
        self.buttonStyle(LiquidGlassButtonStyle(cornerRadius: 8, isProminent: true))
    }

    /// Claude Desktop style floating composer container with elevated glass elevation.
    /// Best practice: system glass on Tahoe, elevated manual stack below.
    @ViewBuilder
    public func floatingComposerCard() -> some View {
        if #available(macOS 26.0, *) {
            self.glassEffect(.regular, in: RoundedRectangle(cornerRadius: 18, style: .continuous))
                .shadow(color: Color.black.opacity(0.18), radius: 24, x: 0, y: 10)
                .shadow(color: Color.black.opacity(0.08), radius: 6, x: 0, y: 2)
        } else {
            self
                .background(
                    RoundedRectangle(cornerRadius: 18, style: .continuous)
                        .fill(.ultraThinMaterial)
                        .overlay(LiquidGlass.specularSheen(cornerRadius: 18))
                        .overlay(LiquidGlass.specularBorder(cornerRadius: 18, isHovered: false))
                )
                .clipShape(RoundedRectangle(cornerRadius: 18, style: .continuous))
                .shadow(color: Color.black.opacity(0.18), radius: 24, x: 0, y: 10)
                .shadow(color: Color.black.opacity(0.08), radius: 6, x: 0, y: 2)
        }
    }

    /// Claude Desktop style interactive model picker pill.
    public func claudeModelPill(isHovered: Bool = false) -> some View {
        self
            .padding(.horizontal, 12)
            .padding(.vertical, 6)
            .background(
                Capsule(style: .continuous)
                    .fill(.ultraThinMaterial)
                    .overlay(
                        Capsule(style: .continuous)
                            .strokeBorder(
                                LinearGradient(
                                    colors: [
                                        Color.white.opacity(isHovered ? 0.35 : 0.20),
                                        Color.white.opacity(isHovered ? 0.15 : 0.05)
                                    ],
                                    startPoint: .topLeading,
                                    endPoint: .bottomTrailing
                                ),
                                lineWidth: 1
                            )
                    )
            )
            .clipShape(Capsule(style: .continuous))
            .shadow(color: Color.black.opacity(0.06), radius: 4, x: 0, y: 2)
    }

    /// Liquid Glass styling for thinking/reasoning disclosures (Claude Desktop / DeepSeek pattern).
    public func liquidGlassThinkingCard() -> some View {
        self
            .background(
                RoundedRectangle(cornerRadius: 12, style: .continuous)
                    .fill(Color.purple.opacity(0.04))
                    .background(.ultraThinMaterial)
                    .overlay(
                        RoundedRectangle(cornerRadius: 12, style: .continuous)
                            .strokeBorder(
                                LinearGradient(
                                    colors: [
                                        Color.purple.opacity(0.35),
                                        Color.indigo.opacity(0.15),
                                        Color.clear,
                                        Color.purple.opacity(0.20)
                                    ],
                                    startPoint: .topLeading,
                                    endPoint: .bottomTrailing
                                ),
                                lineWidth: 1
                            )
                    )
            )
            .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
            .shadow(color: Color.black.opacity(0.06), radius: 6, x: 0, y: 3)
    }

    /// Liquid Glass styling for floating modal sheets and dialogs.
    /// Best practice: system glass on Tahoe, manual stack below.
    @ViewBuilder
    public func liquidGlassModalCard(cornerRadius: CGFloat = 16) -> some View {
        if #available(macOS 26.0, *) {
            self.glassEffect(.regular, in: RoundedRectangle(cornerRadius: cornerRadius, style: .continuous))
                .shadow(color: Color.black.opacity(0.24), radius: 32, x: 0, y: 12)
                .shadow(color: Color.black.opacity(0.08), radius: 8, x: 0, y: 2)
        } else {
            self
                .background(
                    ZStack {
                        RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
                            .fill(.ultraThinMaterial)
                        LiquidGlass.specularSheen(cornerRadius: cornerRadius)
                        LiquidGlass.specularBorder(cornerRadius: cornerRadius, isHovered: false)
                    }
                )
                .clipShape(RoundedRectangle(cornerRadius: cornerRadius, style: .continuous))
                .shadow(color: Color.black.opacity(0.24), radius: 32, x: 0, y: 12)
                .shadow(color: Color.black.opacity(0.08), radius: 8, x: 0, y: 2)
        }
    }

    /// Liquid Glass styling for text input fields.
    public func liquidGlassTextField(cornerRadius: CGFloat = 8, isFocused: Bool = false) -> some View {
        self
            .padding(.horizontal, 10)
            .padding(.vertical, 7)
            .background(
                ZStack {
                    RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
                        .fill(Color.white.opacity(0.06))
                    RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
                        .strokeBorder(
                            isFocused
                                ? LinearGradient(colors: [Color.accentColor, Color.purple], startPoint: .topLeading, endPoint: .bottomTrailing)
                                : LinearGradient(colors: [Color.white.opacity(0.18), Color.white.opacity(0.06)], startPoint: .topLeading, endPoint: .bottomTrailing),
                            lineWidth: isFocused ? 1.5 : 1
                        )
                }
            )
            .clipShape(RoundedRectangle(cornerRadius: cornerRadius, style: .continuous))
    }
}

// MARK: - Apple Liquid Glass Loop Logo (Mathematical Lemniscate)

public struct LemniscateLoopShape: Shape {
    public init() {}

    public func path(in rect: CGRect) -> Path {
        var path = Path()
        let midX = rect.midX
        let midY = rect.midY
        let a = (rect.width / 2.0) * 0.88
        let yScale = (rect.height / 2.0) * 1.55

        let steps = 144
        for i in 0...steps {
            let t = (Double(i) / Double(steps)) * 2.0 * .pi
            let sinT = sin(t)
            let cosT = cos(t)
            let denom = 1.0 + sinT * sinT
            let x = midX + CGFloat((a * cosT) / denom)
            let y = midY - CGFloat((Double(yScale) * sinT * cosT) / denom)

            if i == 0 {
                path.move(to: CGPoint(x: x, y: y))
            } else {
                path.addLine(to: CGPoint(x: x, y: y))
            }
        }
        path.closeSubpath()
        return path
    }
}

public struct LACLogoView: View {
    public let size: CGFloat
    public var isAnimated: Bool

    @State private var phase: CGFloat = 0
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private var animated: Bool { isAnimated && !reduceMotion }

    public init(size: CGFloat = 28, isAnimated: Bool = true) {
        self.size = size
        self.isAnimated = isAnimated
    }

    private var loopGradient: LinearGradient {
        LinearGradient(
            stops: [
                .init(color: Color(red: 0.12, green: 0.55, blue: 1.00), location: 0.0), // Electric Cobalt
                .init(color: Color(red: 0.00, green: 0.85, blue: 0.95), location: 0.35), // Cyan Laser
                .init(color: Color(red: 0.75, green: 0.35, blue: 1.00), location: 0.70), // Vivid Violet
                .init(color: Color(red: 1.00, green: 0.25, blue: 0.55), location: 1.0)  // Magenta Sheen
            ],
            startPoint: .topLeading,
            endPoint: .bottomTrailing
        )
    }

    public var body: some View {
        ZStack {
            // Ambient neon glow layer
            LemniscateLoopShape()
                .stroke(
                    loopGradient,
                    style: StrokeStyle(lineWidth: max(2.5, size * 0.18), lineCap: .round, lineJoin: .round)
                )
                .blur(radius: max(1.5, size * 0.12))
                .opacity(animated ? (0.65 + 0.25 * sin(phase)) : 0.75)

            // Optical glass core conduit
            LemniscateLoopShape()
                .stroke(
                    loopGradient,
                    style: StrokeStyle(lineWidth: max(2.0, size * 0.14), lineCap: .round, lineJoin: .round)
                )

            // Specular refraction highlight
            LemniscateLoopShape()
                .stroke(
                    LinearGradient(
                        colors: [
                            Color.white.opacity(0.85),
                            Color.white.opacity(0.10),
                            Color.clear,
                            Color.white.opacity(0.60)
                        ],
                        startPoint: .top,
                        endPoint: .bottom
                    ),
                    style: StrokeStyle(lineWidth: max(1.0, size * 0.06), lineCap: .round, lineJoin: .round)
                )

            // Central crossing highlight sparkle
            Circle()
                .fill(Color.white.opacity(0.95))
                .frame(width: max(2.5, size * 0.09), height: max(2.5, size * 0.09))
                .blur(radius: max(0.5, size * 0.02))
                .scaleEffect(animated ? (1.0 + 0.2 * cos(phase)) : 1.0)
        }
        .frame(width: size, height: size * 0.65)
        .onAppear {
            if animated {
                withAnimation(Animation.easeInOut(duration: 2.4).repeatForever(autoreverses: true)) {
                    phase = .pi * 2
                }
            }
        }
    }
}

// MARK: - Liquid Glass Loop Avatar

public struct LACAvatarView: View {
    public let diameter: CGFloat
    public var isPulsing: Bool

    public init(diameter: CGFloat = 28, isPulsing: Bool = false) {
        self.diameter = diameter
        self.isPulsing = isPulsing
    }

    public var body: some View {
        ZStack {
            Circle()
                .fill(
                    LinearGradient(
                        colors: [
                            Color(red: 0.12, green: 0.55, blue: 1.00).opacity(0.28),
                            Color(red: 0.75, green: 0.35, blue: 1.00).opacity(0.14)
                        ],
                        startPoint: .topLeading,
                        endPoint: .bottomTrailing
                    )
                )
                .overlay(
                    Circle()
                        .strokeBorder(
                            LinearGradient(
                                colors: [Color.white.opacity(0.40), Color.white.opacity(0.10)],
                                startPoint: .topLeading,
                                endPoint: .bottomTrailing
                            ),
                            lineWidth: 1
                        )
                )
                .shadow(color: Color.accentColor.opacity(0.18), radius: 6, x: 0, y: 2)

            LACLogoView(size: diameter * 0.72, isAnimated: isPulsing)
        }
        .frame(width: diameter, height: diameter)
    }
}

// MARK: - Executive Thought Box for <think> Reasoning

public struct ExecutiveThoughtBox: View {
    public let thought: String
    public var isStreaming: Bool
    @State private var isExpanded: Bool
    @State private var userToggled: Bool = false

    public init(thought: String, isStreaming: Bool = false) {
        self.thought = thought
        self.isStreaming = isStreaming
        self._isExpanded = State(initialValue: isStreaming)
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Button {
                LiquidGlass.haptic(.alignment)
                userToggled = true
                withAnimation(LiquidGlass.spring) {
                    isExpanded.toggle()
                }
            } label: {
                HStack(spacing: 6) {
                    Image(systemName: isExpanded ? "chevron.down" : "chevron.right")
                        .font(.system(size: 9, weight: .bold))
                        .foregroundColor(.secondary)

                    Image(systemName: "brain.head.profile")
                        .font(.system(size: 11, weight: .medium))
                        .foregroundColor(.purple)

                    Text(isStreaming ? "Thinking..." : "Thought process")
                        .font(.system(size: 11.5, weight: .semibold))
                        .foregroundColor(.primary)

                    if isStreaming {
                        ProgressView()
                            .controlSize(.mini)
                    }

                    Spacer()

                    Text("\(thought.split(separator: " ").count) words")
                        .font(.system(size: 10, design: .monospaced))
                        .foregroundColor(.secondary.opacity(0.7))
                }
                .padding(.horizontal, 10)
                .padding(.vertical, 7)
                .background(
                    RoundedRectangle(cornerRadius: 8, style: .continuous)
                        .fill(Color.purple.opacity(0.06))
                        .overlay(
                            RoundedRectangle(cornerRadius: 8, style: .continuous)
                                .strokeBorder(Color.purple.opacity(0.20), lineWidth: 1)
                        )
                )
            }
            .buttonStyle(.plain)

            if isExpanded {
                ScrollView {
                    Text(thought)
                        .font(.system(size: 11.5, design: .monospaced))
                        .foregroundColor(.secondary)
                        .lineSpacing(2.5)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(10)
                        .textSelection(.enabled)
                }
                .frame(maxHeight: 220)
                .background(
                    RoundedRectangle(cornerRadius: 8, style: .continuous)
                        .fill(Color.black.opacity(0.18))
                        .overlay(
                            RoundedRectangle(cornerRadius: 8, style: .continuous)
                                .strokeBorder(Color.white.opacity(0.08), lineWidth: 1)
                        )
                )
                .transition(.opacity.combined(with: .move(edge: .top)))
            }
        }
        .padding(.vertical, 4)
        .onChange(of: isStreaming) { streaming in
            if !streaming && !userToggled {
                withAnimation(LiquidGlass.spring) {
                    isExpanded = false
                }
            }
        }
    }
}


