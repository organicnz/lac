import AppKit
import SwiftUI
import UniformTypeIdentifiers

// MARK: - Attached Context File (Codex & Claude Desktop Context Injection)

public struct AttachedContextFile: Identifiable, Hashable, Sendable {
    public let id = UUID()
    public let name: String
    public let path: String
    public let content: String
    public let byteCount: Int
    /// True when `content` was truncated to the 128 KB injection cap.
    public let wasTruncated: Bool

    public var formattedSize: String {
        if byteCount >= 1024 * 1024 {
            return String(format: "%.1f MB", Double(byteCount) / (1024.0 * 1024.0))
        } else if byteCount >= 1024 {
            return String(format: "%.1f KB", Double(byteCount) / 1024.0)
        }
        return "\(byteCount) B"
    }
}

// MARK: - Slash Commands (Codex & Claude Desktop Prompt Acceleration)

public struct SlashCommandItem: Identifiable, Hashable, Sendable {
    public let command: String
    public let title: String
    public let description: String
    public let icon: String
    public let template: String
    public var id: String { command }
}

@MainActor
public let availableSlashCommands: [SlashCommandItem] = [
    SlashCommandItem(
        command: "/explain",
        title: "Explain Concept / Code",
        description: "Step-by-step reasoning on architecture, algorithms, and tradeoffs",
        icon: "sparkles",
        template: "Explain the following code or concept in detail with step-by-step reasoning:\n\n"
    ),
    SlashCommandItem(
        command: "/refactor",
        title: "Refactor for Apple Silicon",
        description: "Idiomatic clean rewrite for unified memory efficiency and speed",
        icon: "arrow.triangle.2.circlepath",
        template: "Refactor the following code for idiomatic architecture, memory efficiency, and high performance:\n\n"
    ),
    SlashCommandItem(
        command: "/test",
        title: "Generate Unit Tests",
        description: "Comprehensive test suite covering edge cases, happy paths, and concurrency",
        icon: "checkmark.shield",
        template: "Generate comprehensive unit tests covering edge cases, failure states, and concurrency:\n\n"
    ),
    SlashCommandItem(
        command: "/audit",
        title: "Security & Resource Audit",
        description: "Audit for leaks, race conditions, socket stalls, and security flaws",
        icon: "exclamationmark.shield",
        template: "Perform a deep audit on the following code, checking for memory leaks, data races, socket stalls, and security vulnerabilities:\n\n"
    ),
    SlashCommandItem(
        command: "/bench",
        title: "Benchmark & Complexity",
        description: "Analyze Big-O runtime, memory complexity, and Apple Silicon SIMD potential",
        icon: "speedometer",
        template: "Analyze the time and space complexity of this code and suggest hardware-accelerated optimizations:\n\n"
    ),
    SlashCommandItem(
        command: "/fix",
        title: "Diagnose & Fix Bug",
        description: "Root cause analysis with surgical drop-in corrected code",
        icon: "wrench.and.screwdriver",
        template: "Diagnose and fix the following issue. Identify the root cause and provide the corrected drop-in code:\n\n"
    )
]

// MARK: - Chat: Claude Desktop & Codex agentic prompting canvas

public struct ChatView: View {
    @EnvironmentObject private var store: ChatStore
    @EnvironmentObject private var network: NetworkManager
    @Binding var sidebarVisibility: NavigationSplitViewVisibility
    @State private var draft = ""
    @State private var attachedFiles: [AttachedContextFile] = []
    @State private var isCustomModelPresented = false
    @State private var customModelText = ""
    @State private var isStartingBackend = false
    @State private var isDropTargeted = false
    @State private var isTuningPresented = false
    @State private var attachNotice: String?
    @State private var exportError: String?
    public var onOpenDashboard: (() -> Void)?
    public var onOpenModelHub: (() -> Void)?
    public var onOpenCodeAssistant: ((String, String) -> Void)?

    public init(
        sidebarVisibility: Binding<NavigationSplitViewVisibility> = .constant(.all),
        onOpenDashboard: (() -> Void)? = nil,
        onOpenModelHub: (() -> Void)? = nil,
        onOpenCodeAssistant: ((String, String) -> Void)? = nil
    ) {
        self._sidebarVisibility = sidebarVisibility
        self.onOpenDashboard = onOpenDashboard
        self.onOpenModelHub = onOpenModelHub
        self.onOpenCodeAssistant = onOpenCodeAssistant
    }

    public var body: some View {
        VStack(spacing: 0) {
            claudeHeaderBar
            Divider().opacity(0.3)

            ZStack(alignment: .bottom) {
                transcriptScrollView
                    .frame(maxWidth: .infinity, maxHeight: .infinity)

                floatingComposer
                    .padding(.horizontal, 20)
                    .padding(.bottom, 16)
            }
        }
        .background(VisualEffectView().ignoresSafeArea())
        .onAppear {
            Task { await store.fetchModels() }
        }
        .alert("Export failed", isPresented: Binding(
            get: { exportError != nil },
            set: { if !$0 { exportError = nil } }
        )) {
            Button("OK", role: .cancel) { exportError = nil }
        } message: {
            Text(exportError ?? "Unknown error")
        }
    }

    // MARK: Claude-Style Header Bar

    private var claudeHeaderBar: some View {
        HStack(spacing: 12) {
            // Sidebar Toggle Button (Claude Desktop / Codex pattern)
            Button {
                withAnimation(LiquidGlass.spring) {
                    sidebarVisibility = (sidebarVisibility == .detailOnly) ? .all : .detailOnly
                }
                LiquidGlass.haptic(.alignment)
            } label: {
                Image(systemName: "sidebar.leading")
                    .font(.system(size: 13, weight: .medium))
                    .foregroundColor(.secondary)
                    .frame(width: 28, height: 28)
                    .background(
                        RoundedRectangle(cornerRadius: 6, style: .continuous)
                            .fill(Color.white.opacity(0.06))
                    )
            }
            .buttonStyle(.plain)
            .help("Toggle Sidebar (⌘B)")
            .keyboardShortcut("b", modifiers: .command)

            VStack(alignment: .leading, spacing: 2) {
                Text(store.activeThread?.title ?? "New Chat")
                    .font(.system(size: 13, weight: .semibold))
                    .lineLimit(1)
                Text("Local Agentic Coding Gateway")
                    .font(.system(size: 10))
                    .foregroundColor(.secondary)
            }
            .frame(maxWidth: 200, alignment: .leading)

            Spacer()

            // Claude Desktop Model Selector Pill & Tuning
            HStack(spacing: 6) {
                modelPickerPill

                Button {
                    isTuningPresented.toggle()
                    LiquidGlass.haptic(.alignment)
                } label: {
                    Image(systemName: "slider.horizontal.3")
                        .font(.system(size: 11, weight: .medium))
                        .foregroundColor(store.thinkingMode ? .accentColor : .secondary)
                        .frame(width: 26, height: 26)
                        .background(
                            RoundedRectangle(cornerRadius: 6, style: .continuous)
                                .fill(store.thinkingMode ? Color.accentColor.opacity(0.18) : Color.white.opacity(0.06))
                        )
                }
                .buttonStyle(.plain)
                .popover(isPresented: $isTuningPresented, arrowEdge: .bottom) {
                    SystemTuningPopoverView(store: store)
                }
                .help("Model Hyperparameters & System Tuning (AGENTS.md)")
            }

            Spacer()

            HStack(spacing: 8) {
                // Clear thread button
                if let thread = store.activeThread, !thread.messages.isEmpty {
                    Button {
                        LiquidGlass.haptic(.alignment)
                        withAnimation(LiquidGlass.spring) {
                            store.clearActiveThread()
                        }
                    } label: {
                        HStack(spacing: 4) {
                            Image(systemName: "arrow.counterclockwise")
                                .font(.system(size: 10))
                            Text("Clear")
                                .font(.system(size: 11, weight: .medium))
                        }
                    }
                    .controlSize(.small)
                    .lacGlass()
                    .help("Clear messages in this conversation")

                    Menu {
                        Button {
                            copyMarkdownExport()
                        } label: {
                            Label("Copy as Markdown", systemImage: "doc.on.doc")
                        }

                        Button {
                            saveMarkdownFile()
                        } label: {
                            Label("Save to File (.md)...", systemImage: "arrow.down.doc")
                        }
                    } label: {
                        HStack(spacing: 4) {
                            Image(systemName: "square.and.arrow.up")
                                .font(.system(size: 10))
                            Text("Export")
                                .font(.system(size: 11, weight: .medium))
                        }
                    }
                    .menuStyle(.borderlessButton)
                    .fixedSize()
                    .controlSize(.small)
                    .lacGlass()
                    .help("Export conversation as Markdown")
                }

                if let onOpenDashboard {
                    Button {
                        LiquidGlass.haptic(.alignment)
                        onOpenDashboard()
                    } label: {
                        HStack(spacing: 5) {
                            Image(systemName: "gauge.with.dots.needle.bottom.50percent")
                                .font(.system(size: 11))
                            Text("Ops")
                                .font(.system(size: 11, weight: .medium))
                        }
                    }
                    .controlSize(.small)
                    .lacGlass()
                    .help("View Ops Dashboard")
                }

                Button {
                    LiquidGlass.haptic(.alignment)
                    _ = store.newThread()
                } label: {
                    Image(systemName: "square.and.pencil")
                        .font(.system(size: 11))
                }
                .controlSize(.small)
                .lacGlass()
                .help("New Chat (⌘N)")
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
        .background(.ultraThinMaterial)
    }

    // MARK: Model Picker Dropdown Pill

    private var modelPickerPill: some View {
        Menu {
            if store.models.isEmpty {
                Button("mlx-community/Qwen3.8-27B-4bit (Default)") {
                    store.selectedModel = "mlx-community/Qwen3.8-27B-4bit"
                }
                Button("Qwen/Qwen2.5-Coder-32B-Instruct") {
                    store.selectedModel = "Qwen/Qwen2.5-Coder-32B-Instruct"
                }
            } else {
                Section("Detected Local Models") {
                    ForEach(store.models, id: \.self) { m in
                        Button {
                            store.selectedModel = m
                        } label: {
                            HStack {
                                Text(m)
                                if store.selectedModel == m {
                                    Image(systemName: "checkmark")
                                }
                            }
                        }
                    }
                }
            }

            Divider()

            if let onOpenModelHub {
                Button {
                    onOpenModelHub()
                } label: {
                    HStack {
                        Image(systemName: "magnifyingglass")
                        Text("Browse Hugging Face Models (LM Studio)...")
                    }
                }
            }

            Button("Enter Custom Model...") {
                customModelText = store.selectedModel
                isCustomModelPresented = true
            }

            Button("Refresh Local Models") {
                Task { await store.fetchModels() }
            }
        } label: {
            HStack(spacing: 7) {
                LACLogoView(size: 14)

                Text(store.friendlyModelName)
                    .font(.system(size: 12, weight: .medium))
                    .lineLimit(1)

                Text("64k")
                    .font(.system(size: 9, weight: .semibold, design: .monospaced))
                    .padding(.horizontal, 4)
                    .padding(.vertical, 1)
                    .background(Capsule().fill(Color.white.opacity(0.12)))
                    .foregroundColor(.secondary)

                Image(systemName: "chevron.down")
                    .font(.system(size: 9, weight: .semibold))
                    .foregroundColor(.secondary)
            }
            .claudeModelPill()
        }
        .menuStyle(.borderlessButton)
        .fixedSize()
        .sheet(isPresented: $isCustomModelPresented) {
            VStack(spacing: 16) {
                Text("Custom Model ID")
                    .font(.headline)
                TextField("e.g. mlx-community/Qwen3.8-27B-4bit", text: $customModelText)
                    .textFieldStyle(.roundedBorder)
                    .frame(width: 320)
                HStack {
                    Button("Cancel") { isCustomModelPresented = false }
                    Button("Set Model") {
                        store.selectedModel = customModelText
                        isCustomModelPresented = false
                    }
                    .lacGlassProminent()
                }
            }
            .padding(24)
        }
    }

    // MARK: Centered Conversation Reading Column (780pt Claude standard)

    private var transcriptScrollView: some View {
        ScrollViewReader { proxy in
            ScrollView {
                VStack(spacing: 24) {
                    if let thread = store.activeThread, !thread.messages.isEmpty {
                        ForEach(Array(thread.messages.enumerated()), id: \.element.id) { idx, msg in
                            let isLastAssistant = (idx == thread.messages.count - 1) && (msg.role == "assistant")
                            ClaudeMessageRow(
                                message: msg,
                                isLastAssistant: isLastAssistant,
                                onOpenCodeAssistant: onOpenCodeAssistant,
                                onEditPrompt: { prompt in
                                    draft = prompt
                                    LiquidGlass.haptic(.alignment)
                                }
                            )
                        }
                    } else if !store.isSending {
                        claudeEmptyState
                    }

                    // Thinking / Generating state
                    if store.isSending && store.streamText.isEmpty {
                        HStack(alignment: .top, spacing: 14) {
                            claudeAvatar
                            HStack(spacing: 8) {
                                ProgressView().scaleEffect(0.7)
                                Text("Thinking...")
                                    .font(.system(size: 13, weight: .medium))
                                    .foregroundColor(.secondary)
                            }
                            .padding(.horizontal, 14)
                            .padding(.vertical, 10)
                            .liquidGlassCard(cornerRadius: 12, hoverable: false)
                            Spacer()
                        }
                        .id("thinking")
                    }

                    // Active streaming token response
                    if !store.streamText.isEmpty {
                        ClaudeMessageRow(
                            message: ChatMessage(role: "assistant", content: store.streamText),
                            isStreaming: true,
                            onOpenCodeAssistant: onOpenCodeAssistant
                        )
                        .id("streaming")
                    }

                    // Self-healing interactive error card
                    if let err = store.errorText {
                        VStack(alignment: .leading, spacing: 10) {
                            HStack(alignment: .top, spacing: 8) {
                                Image(systemName: "exclamationmark.triangle.fill")
                                    .foregroundColor(.orange)
                                    .font(.system(size: 14))
                                VStack(alignment: .leading, spacing: 4) {
                                    Text("Gateway / Backend Notice")
                                        .font(.system(size: 13, weight: .semibold))
                                        .foregroundColor(.primary)
                                    Text(err)
                                        .font(.system(size: 12))
                                        .foregroundColor(.secondary)
                                }
                            }

                            HStack(spacing: 8) {
                                if network.response == nil {
                                    Button {
                                        LiquidGlass.haptic(.alignment)
                                        isStartingBackend = true
                                        Task {
                                            await network.startRouter()
                                            isStartingBackend = false
                                            store.retryLastPrompt()
                                        }
                                    } label: {
                                        HStack(spacing: 5) {
                                            if isStartingBackend {
                                                ProgressView().scaleEffect(0.6)
                                            } else {
                                                Image(systemName: "bolt.fill")
                                                    .font(.system(size: 9))
                                            }
                                            Text("Start LAC Router Gateway")
                                                .font(.system(size: 11, weight: .semibold))
                                        }
                                    }
                                    .controlSize(.small)
                                    .lacGlassProminent()
                                } else {
                                    Button {
                                        LiquidGlass.haptic(.alignment)
                                        isStartingBackend = true
                                        Task {
                                            await network.startMLX()
                                            isStartingBackend = false
                                            store.retryLastPrompt()
                                        }
                                    } label: {
                                        HStack(spacing: 5) {
                                            if isStartingBackend {
                                                ProgressView().scaleEffect(0.6)
                                            } else {
                                                Image(systemName: "play.fill")
                                                    .font(.system(size: 9))
                                            }
                                            Text("Start MLX Backend (Qwen 3.8)")
                                                .font(.system(size: 11, weight: .medium))
                                        }
                                    }
                                    .controlSize(.small)
                                    .lacGlassProminent()
                                }

                                Button {
                                    LiquidGlass.haptic(.alignment)
                                    store.retryLastPrompt()
                                } label: {
                                    HStack(spacing: 5) {
                                        Image(systemName: "arrow.clockwise")
                                            .font(.system(size: 10))
                                        Text("Retry Prompt")
                                            .font(.system(size: 11, weight: .medium))
                                    }
                                }
                                .controlSize(.small)
                                .lacGlass()

                                if let onOpenDashboard {
                                    Button {
                                        LiquidGlass.haptic(.alignment)
                                        onOpenDashboard()
                                    } label: {
                                        Text("Open Ops Dashboard")
                                            .font(.system(size: 11, weight: .medium))
                                    }
                                    .controlSize(.small)
                                    .lacGlass()
                                }
                            }
                        }
                        .padding(14)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .liquidGlassCard(cornerRadius: 12, hoverable: false, tint: .orange)
                    }

                    Color.clear.frame(height: 110).id("bottomSpacer")
                }
                .frame(maxWidth: 780)
                .frame(maxWidth: .infinity)
                .padding(.horizontal, 24)
                .padding(.top, 28)
                .id("transcript")
            }
            .onChange(of: store.activeThread?.messages.count ?? 0) { _ in
                withAnimation { proxy.scrollTo("bottomSpacer", anchor: .bottom) }
            }
            .onChange(of: store.isSending) { sending in
                withAnimation {
                    if sending {
                        proxy.scrollTo("thinking", anchor: .bottom)
                    } else {
                        proxy.scrollTo("bottomSpacer", anchor: .bottom)
                    }
                }
            }
            .onChange(of: store.streamText) { _ in
                proxy.scrollTo("streaming", anchor: .bottom)
            }
        }
    }

    // MARK: Claude Desktop Empty State with Suggestions

    private var claudeEmptyState: some View {
        VStack(spacing: 24) {
            VStack(spacing: 14) {
                ZStack {
                    Circle()
                        .fill(
                            RadialGradient(
                                gradient: Gradient(colors: [
                                    Color.accentColor.opacity(0.30),
                                    Color.purple.opacity(0.12),
                                    Color.clear
                                ]),
                                center: .center,
                                startRadius: 0,
                                endRadius: 44
                            )
                        )
                        .frame(width: 80, height: 80)
                        .overlay(
                            Circle()
                                .strokeBorder(
                                    LinearGradient(
                                        colors: [Color.white.opacity(0.35), Color.white.opacity(0.08)],
                                        startPoint: .topLeading,
                                        endPoint: .bottomTrailing
                                    ),
                                    lineWidth: 1
                                )
                        )
                        .shadow(color: Color.accentColor.opacity(0.20), radius: 16, x: 0, y: 6)

                    LACLogoView(size: 52, isAnimated: true)
                }

                Text("Loop LAC Studio")
                    .font(.system(size: 22, weight: .bold, design: .rounded))
                    .foregroundColor(.primary)

                Text("Autonomous Local Agentic Coding & Intelligence on Apple Silicon\n100% on-device weights via gateway :\(store.port) · Zero cloud telemetry")
                    .font(.system(size: 13))
                    .foregroundColor(.secondary)
                    .multilineTextAlignment(.center)
                    .lineSpacing(3)
            }
            .padding(.top, 40)

            // 2x2 Suggestion Cards
            LazyVGrid(columns: [GridItem(.flexible(), spacing: 12), GridItem(.flexible(), spacing: 12)], spacing: 12) {
                suggestionCard(
                    icon: "memorychip",
                    title: "MLX KV Caching",
                    subtitle: "Explain FP16/INT8 cache retention on Apple Silicon",
                    prompt: "Explain how the MLX KV cache works on Apple Silicon unified memory and how to prevent memory fragmentation during long generation loops."
                )
                suggestionCard(
                    icon: "gauge.with.dots.needle.bottom.50percent",
                    title: "Gateway Health & Latencies",
                    subtitle: "Analyze EWMA response times and inflight limits",
                    prompt: "How does the LAC gateway route requests between MLX, llama.cpp, and Ollama, and how does it calculate EWMA latencies?"
                )
                suggestionCard(
                    icon: "chevron.left.forwardslash.chevron.right",
                    title: "Swift Concurrency Actor",
                    subtitle: "Draft a modern actor with Task isolation",
                    prompt: "Write a high-performance Swift actor for managing asynchronous background tasks with structured concurrency on macOS 14."
                )
                suggestionCard(
                    icon: "speedometer",
                    title: "Context Scaling & Limits",
                    subtitle: "Scale 27B model context to 64K tokens safely",
                    prompt: "What are the recommended KV cache and context cap settings for running Qwen 3.8 27B locally on a 64GB Apple Silicon Mac?"
                )
            }
            .frame(maxWidth: 680)
        }
    }

    private func suggestionCard(icon: String, title: String, subtitle: String, prompt: String) -> some View {
        Button {
            LiquidGlass.haptic(.alignment)
            draft = prompt
            submit()
        } label: {
            VStack(alignment: .leading, spacing: 6) {
                HStack(spacing: 6) {
                    Image(systemName: icon)
                        .font(.system(size: 12, weight: .medium))
                        .foregroundColor(.accentColor)
                    Text(title)
                        .font(.system(size: 12, weight: .semibold))
                        .foregroundColor(.primary)
                    Spacer()
                    Image(systemName: "arrow.up.right")
                        .font(.system(size: 10))
                        .foregroundColor(.secondary.opacity(0.6))
                }
                Text(subtitle)
                    .font(.system(size: 11))
                    .foregroundColor(.secondary)
                    .lineLimit(2)
                    .multilineTextAlignment(.leading)
            }
            .padding(12)
            .liquidGlassCard(cornerRadius: 12, hoverable: true)
        }
        .buttonStyle(.plain)
    }

    // MARK: Floating Bottom Composer (Claude Desktop signature)

    private var isShowingSlashMenu: Bool {
        draft.hasPrefix("/") && !draft.contains(" ") && !draft.contains("\n")
    }

    private var filteredSlashCommands: [SlashCommandItem] {
        let q = draft.lowercased()
        if q == "/" { return availableSlashCommands }
        let filter = q.dropFirst()
        return availableSlashCommands.filter {
            $0.command.dropFirst().lowercased().hasPrefix(filter) || $0.title.lowercased().contains(filter)
        }
    }

    private var slashCommandsPopover: some View {
        VStack(alignment: .leading, spacing: 3) {
            HStack {
                Text("Slash Commands (Codex & Claude Desktop)")
                    .font(.system(size: 10, weight: .bold))
                    .foregroundColor(.secondary)
                Spacer()
                Text("esc or space to dismiss")
                    .font(.system(size: 9))
                    .foregroundColor(.secondary.opacity(0.7))
            }
            .padding(.horizontal, 10)
            .padding(.top, 6)
            .padding(.bottom, 2)

            Divider().opacity(0.2)

            ForEach(filteredSlashCommands) { cmd in
                Button {
                    LiquidGlass.haptic(.alignment)
                    draft = cmd.template
                } label: {
                    HStack(spacing: 8) {
                        Image(systemName: cmd.icon)
                            .font(.system(size: 11, weight: .medium))
                            .foregroundColor(.accentColor)
                            .frame(width: 18)

                        Text(cmd.command)
                            .font(.system(size: 12, weight: .bold, design: .monospaced))
                            .foregroundColor(.primary)

                        Text("— " + cmd.description)
                            .font(.system(size: 11))
                            .foregroundColor(.secondary)
                            .lineLimit(1)

                        Spacer()

                        Image(systemName: "return")
                            .font(.system(size: 9))
                            .foregroundColor(.secondary.opacity(0.6))
                    }
                    .padding(.horizontal, 10)
                    .padding(.vertical, 6)
                    .background(
                        RoundedRectangle(cornerRadius: 6, style: .continuous)
                            .fill(Color.white.opacity(0.04))
                    )
                }
                .buttonStyle(.plain)
            }
        }
        .padding(6)
        .liquidGlassCard(cornerRadius: 12, hoverable: false)
        .frame(maxWidth: 780)
    }

    private func attachFiles() {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = true
        panel.canChooseDirectories = false
        panel.canChooseFiles = true
        panel.allowedContentTypes = [
            UTType.plainText,
            UTType.sourceCode,
            UTType.json,
            UTType.xml,
            UTType.yaml,
            UTType.data
        ]
        if panel.runModal() == .OK {
            for url in panel.urls {
                loadFileAsAttachment(url)
            }
        }
    }

    private func loadFileAsAttachment(_ url: URL) {
        let maxBytes = 128 * 1024
        guard url.startAccessingSecurityScopedResource() || true else { return }
        defer { url.stopAccessingSecurityScopedResource() }
        do {
            let data = try Data(contentsOf: url)
            let slice = data.prefix(maxBytes)
            guard let content = String(data: slice, encoding: .utf8), !content.isEmpty else {
                attachNotice = "\(url.lastPathComponent): not a readable UTF-8 text file — skipped."
                return
            }
            let truncated = data.count > maxBytes
            let item = AttachedContextFile(
                name: url.lastPathComponent,
                path: url.path,
                content: content,
                byteCount: data.count,
                wasTruncated: truncated
            )
            if !attachedFiles.contains(where: { $0.path == item.path }) {
                attachedFiles.append(item)
                LiquidGlass.haptic(.alignment)
            }
            attachNotice = truncated
                ? "\(url.lastPathComponent): truncated to 128 KB of \(item.formattedSize) for context injection."
                : nil
        } catch {
            attachNotice = "\(url.lastPathComponent): could not read file (\(error.localizedDescription))."
        }
    }

    private var floatingComposer: some View {
        VStack(spacing: 6) {
            if isShowingSlashMenu && !filteredSlashCommands.isEmpty {
                slashCommandsPopover
                    .transition(.opacity.combined(with: .move(edge: .bottom)))
            }

            VStack(spacing: 8) {
                if let notice = attachNotice {
                    HStack(spacing: 6) {
                        Image(systemName: "exclamationmark.triangle.fill")
                            .font(.system(size: 10))
                            .foregroundColor(.orange)
                        Text(notice)
                            .font(.system(size: 10.5))
                            .foregroundColor(.secondary)
                            .lineLimit(2)
                        Spacer()
                        Button {
                            attachNotice = nil
                        } label: {
                            Image(systemName: "xmark")
                                .font(.system(size: 9, weight: .bold))
                                .foregroundColor(.secondary)
                        }
                        .buttonStyle(.plain)
                    }
                    .padding(.horizontal, 12)
                    .padding(.top, 8)
                }
                if !attachedFiles.isEmpty {
                    ScrollView(.horizontal, showsIndicators: false) {
                        HStack(spacing: 6) {
                            ForEach(attachedFiles) { file in
                                HStack(spacing: 5) {
                                    Image(systemName: "doc.text.fill")
                                        .font(.system(size: 10))
                                        .foregroundColor(.accentColor)
                                    Text(file.name)
                                        .font(.system(size: 11, weight: .medium))
                                        .lineLimit(1)
                                    Text(file.wasTruncated ? "(\(file.formattedSize) → 128 KB)" : "(\(file.formattedSize))")
                                        .font(.system(size: 9.5))
                                        .foregroundColor(file.wasTruncated ? .orange : .secondary)
                                    Button {
                                        LiquidGlass.haptic(.alignment)
                                        attachedFiles.removeAll { $0.id == file.id }
                                    } label: {
                                        Image(systemName: "xmark.circle.fill")
                                            .font(.system(size: 11))
                                            .foregroundColor(.secondary.opacity(0.7))
                                    }
                                    .buttonStyle(.plain)
                                }
                                .padding(.horizontal, 8)
                                .padding(.vertical, 4)
                                .background(
                                    Capsule().fill(Color.white.opacity(0.08))
                                        .overlay(Capsule().strokeBorder(Color.white.opacity(0.12), lineWidth: 0.5))
                                )
                            }
                        }
                        .padding(.horizontal, 12)
                        .padding(.top, 8)
                    }
                }

                HStack(alignment: .top, spacing: 8) {
                    Button {
                        LiquidGlass.haptic(.alignment)
                        attachFiles()
                    } label: {
                        Image(systemName: "plus.circle")
                            .font(.system(size: 15))
                            .foregroundColor(.secondary)
                            .padding(.leading, 12)
                            .padding(.top, 13)
                    }
                    .buttonStyle(.plain)
                    .help("Attach context files or source code")

                    TextField("Prompt the local model (Return to send, ⇧Return for newline, / for commands)…", text: $draft, axis: .vertical)
                        .textFieldStyle(.plain)
                        .font(.system(size: 13.5))
                        .lineLimit(1...6)
                        .padding(.horizontal, 6)
                        .padding(.top, 12)
                        .padding(.bottom, 4)
                        .onSubmit {
                            if !NSEvent.modifierFlags.contains(.shift) {
                                submit()
                            }
                        }

                    if !draft.isEmpty {
                        Button {
                            draft = ""
                            LiquidGlass.haptic(.alignment)
                        } label: {
                            Image(systemName: "xmark.circle.fill")
                                .font(.system(size: 12))
                                .foregroundColor(.secondary.opacity(0.6))
                                .padding(.trailing, 10)
                                .padding(.top, 12)
                        }
                        .buttonStyle(.plain)
                    }
                }

                HStack(spacing: 8) {
                    // Left status pill
                    HStack(spacing: 5) {
                        Image(systemName: "bolt.horizontal.fill")
                            .font(.system(size: 9))
                            .foregroundColor(.accentColor)
                        Text("Local 64k · MLX")
                            .font(.system(size: 10, weight: .medium))
                            .foregroundColor(.secondary)
                    }
                    .padding(.horizontal, 8)
                    .padding(.vertical, 4)
                    .background(Capsule().fill(Color.white.opacity(0.06)))

                    // Context Token Budget Meter
                    tokenBudgetMeter

                    Spacer()

                    // Send or Stop button
                    if store.isSending {
                        Button {
                            LiquidGlass.haptic(.alignment)
                            store.cancel()
                        } label: {
                            HStack(spacing: 4) {
                                Image(systemName: "stop.circle.fill")
                                    .font(.system(size: 24))
                                    .foregroundColor(.red)
                            }
                        }
                        .buttonStyle(.plain)
                        .help("Stop Generation")
                    } else {
                        let canSend = !draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !attachedFiles.isEmpty
                        Button {
                            submit()
                        } label: {
                            Image(systemName: "arrow.up.circle.fill")
                                .font(.system(size: 26))
                                .foregroundColor(canSend ? .accentColor : .secondary.opacity(0.35))
                        }
                        .buttonStyle(.plain)
                        .disabled(!canSend)
                        .keyboardShortcut(.return, modifiers: .command)
                        .help("Send (⌘Return)")
                    }
                }
                .padding(.horizontal, 12)
                .padding(.bottom, 8)
            }
            .floatingComposerCard()
            .frame(maxWidth: 780)
            .onDrop(of: [UTType.fileURL], isTargeted: $isDropTargeted) { providers in
                for provider in providers {
                    _ = provider.loadObject(ofClass: URL.self) { url, _ in
                        if let url {
                            DispatchQueue.main.async {
                                loadFileAsAttachment(url)
                            }
                        }
                    }
                }
                return true
            }

            Text("Loop LAC Studio · Router :\(store.port) · 100% Private on Apple Silicon")
                .font(.system(size: 10))
                .foregroundColor(.secondary.opacity(0.7))
        }
    }

    private func submit() {
        var text = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        if !attachedFiles.isEmpty {
            var contextPrefix = "Context Files:\n"
            for file in attachedFiles {
                let ext = (file.path as NSString).pathExtension
                contextPrefix += "=== [File: \(file.name)] (\(file.path)) ===\n```\(ext)\n\(file.content)\n```\n\n"
            }
            contextPrefix += "---\n\n"
            text = contextPrefix + text
            attachedFiles = []
        }
        guard !text.isEmpty, !store.isSending else { return }
        draft = ""
        LiquidGlass.haptic(.alignment)
        store.send(text)
    }

    private var claudeAvatar: some View {
        LACAvatarView(diameter: 28, isPulsing: store.isSending)
    }

    // MARK: - Context Token Budget

    private var estimatedTokenCount: Int {
        let msgCount = (store.activeThread?.messages ?? []).reduce(0) { $0 + $1.content.count }
        let draftCount = draft.count
        let attachCount = attachedFiles.reduce(0) { $0 + $1.content.count }
        return max(1, Int(Double(msgCount + draftCount + attachCount) / 3.8))
    }

    private var tokenBudgetMeter: some View {
        let tokens = estimatedTokenCount
        let maxTokens = 65536
        let ratio = min(1.0, Double(tokens) / Double(maxTokens))
        let isWarning = ratio > 0.75
        let isCritical = ratio > 0.90

        return HStack(spacing: 5) {
            GeometryReader { geo in
                ZStack(alignment: .leading) {
                    Capsule()
                        .fill(Color.white.opacity(0.12))
                    Capsule()
                        .fill(isCritical ? Color.red : (isWarning ? Color.orange : Color.accentColor))
                        .frame(width: max(3, geo.size.width * CGFloat(ratio)))
                }
            }
            .frame(width: 32, height: 4)

            Text(tokens >= 1000 ? String(format: "%.1fk / 64k", Double(tokens) / 1000.0) : "\(tokens) / 64k")
                .font(.system(size: 9.5, weight: .medium, design: .monospaced))
                .foregroundColor(isCritical ? .red : (isWarning ? .orange : .secondary))
        }
        .padding(.horizontal, 7)
        .padding(.vertical, 3.5)
        .background(Capsule().fill(Color.white.opacity(0.05)))
        .help("KV Cache Budget: ~\(tokens) estimated tokens out of 64k context")
    }

    // MARK: - Markdown Export

    private func generateMarkdown() -> String {
        guard let thread = store.activeThread else { return "" }
        let formatter = DateFormatter()
        formatter.dateStyle = .medium
        formatter.timeStyle = .short
        var md = "# \(thread.title)\n\n"
        let lastTs = thread.messages.last?.ts ?? UInt64(Date().timeIntervalSince1970)
        let exportDate = Date(timeIntervalSince1970: TimeInterval(lastTs))
        md += "*Exported from Loop LAC Studio on \(formatter.string(from: exportDate))*  \n"
        md += "*Model: \(store.selectedModel)*\n\n---\n\n"

        for msg in thread.messages {
            let sender = msg.role == "user" ? "### 👤 User" : "### 🤖 Loop LAC Studio (\(store.selectedModel))"
            md += "\(sender)\n\n\(msg.content)\n\n---\n\n"
        }
        return md
    }

    private func copyMarkdownExport() {
        let md = generateMarkdown()
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(md, forType: .string)
        LiquidGlass.haptic(.alignment)
    }

    private func saveMarkdownFile() {
        let md = generateMarkdown()
        let panel = NSSavePanel()
        let rawTitle = store.activeThread?.title ?? "conversation"
        let safeTitle = rawTitle.components(separatedBy: CharacterSet.alphanumerics.inverted)
            .filter { !$0.isEmpty }
            .joined(separator: "-")
        panel.nameFieldStringValue = "\(safeTitle.isEmpty ? "conversation" : safeTitle).md"
        panel.allowedContentTypes = [UTType.plainText]

        panel.begin { response in
            if response == .OK, let url = panel.url {
                do {
                    try md.write(to: url, atomically: true, encoding: .utf8)
                    LiquidGlass.haptic(.alignment)
                } catch {
                    exportError = "Export failed: \(error.localizedDescription)"
                }
            }
        }
    }
}

// MARK: - Claude Message Row (User vs Assistant)

struct ClaudeMessageRow: View {
    @EnvironmentObject var store: ChatStore
    let message: ChatMessage
    var isStreaming: Bool = false
    var isLastAssistant: Bool = false
    var onOpenCodeAssistant: ((String, String) -> Void)? = nil
    var onEditPrompt: ((String) -> Void)? = nil
    @State private var copiedFull = false
    @State private var isHovered = false

    private var timestampString: String {
        let date = Date(timeIntervalSince1970: TimeInterval(message.ts))
        let formatter = DateFormatter()
        formatter.timeStyle = .short
        return formatter.string(from: date)
    }

    var body: some View {
        if message.role == "user" {
            // User Message (clean right-aligned bubble with subtle glass)
            HStack(alignment: .bottom, spacing: 8) {
                Spacer(minLength: 60)

                // Quick hover actions toolbar
                if isHovered {
                    HStack(spacing: 4) {
                        if let onEditPrompt {
                            Button {
                                LiquidGlass.haptic(.alignment)
                                onEditPrompt(message.content)
                            } label: {
                                HStack(spacing: 3) {
                                    Image(systemName: "pencil")
                                        .font(.system(size: 9))
                                    Text("Edit")
                                        .font(.system(size: 9.5))
                                }
                                .foregroundColor(.secondary)
                                .padding(.horizontal, 6)
                                .padding(.vertical, 3)
                                .background(Capsule().fill(Color.white.opacity(0.08)))
                            }
                            .buttonStyle(.plain)
                            .help("Load this prompt back into the composer to edit and resend")
                        }

                        Button {
                            store.forkThread(at: message.id)
                        } label: {
                            HStack(spacing: 3) {
                                Image(systemName: "tuningfork")
                                    .font(.system(size: 9))
                                Text("Fork")
                                    .font(.system(size: 9.5))
                            }
                            .foregroundColor(.secondary)
                            .padding(.horizontal, 6)
                            .padding(.vertical, 3)
                            .background(Capsule().fill(Color.white.opacity(0.08)))
                        }
                        .buttonStyle(.plain)
                        .help("Fork thread into a new conversation from this message")

                        Button {
                            NSPasteboard.general.clearContents()
                            NSPasteboard.general.setString(message.content, forType: .string)
                            LiquidGlass.haptic(.alignment)
                            withAnimation(LiquidGlass.spring) { copiedFull = true }
                            DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) {
                                withAnimation(LiquidGlass.spring) { copiedFull = false }
                            }
                        } label: {
                            HStack(spacing: 3) {
                                Image(systemName: copiedFull ? "checkmark" : "doc.on.doc")
                                    .font(.system(size: 9))
                                Text(copiedFull ? "Copied" : "Copy")
                                    .font(.system(size: 9.5))
                            }
                            .foregroundColor(copiedFull ? .green : .secondary)
                            .padding(.horizontal, 6)
                            .padding(.vertical, 3)
                            .background(Capsule().fill(Color.white.opacity(0.08)))
                        }
                        .buttonStyle(.plain)
                    }
                    .transition(.opacity)
                }

                VStack(alignment: .trailing, spacing: 4) {
                    Text(message.content)
                        .font(.system(size: 13.5))
                        .foregroundColor(.primary)
                        .padding(.horizontal, 16)
                        .padding(.vertical, 11)
                        .background(
                            RoundedRectangle(cornerRadius: 16, style: .continuous)
                                .fill(Color.accentColor.opacity(0.18))
                                .overlay(LiquidGlass.specularBorder(cornerRadius: 16, isHovered: isHovered))
                        )
                        .textSelection(.enabled)

                    Text(timestampString)
                        .font(.system(size: 10))
                        .foregroundColor(.secondary.opacity(0.7))
                        .padding(.trailing, 4)
                }
            }
            .frame(maxWidth: .infinity, alignment: .trailing)
            .onHover { isHovered = $0 }
        } else {
            // Assistant Message (Claude style with Loop avatar, full prose, thought boxes, and code blocks)
            HStack(alignment: .top, spacing: 14) {
                LACAvatarView(diameter: 28, isPulsing: isStreaming)

                VStack(alignment: .leading, spacing: 6) {
                    HStack(spacing: 6) {
                        Text(store.friendlyName(for: message.model ?? store.selectedModel))
                            .font(.system(size: 12, weight: .bold))
                            .foregroundColor(.primary)

                        let isLlama = (message.model ?? store.selectedModel).contains("llama") || (message.model ?? store.selectedModel).contains("8bit") || (message.model ?? store.selectedModel).contains("GGUF")
                        Text(isLlama ? "Local llama" : "Local MLX")
                            .font(.system(size: 9.5, weight: .semibold))
                            .foregroundColor(.secondary)
                            .padding(.horizontal, 5)
                            .padding(.vertical, 1)
                            .background(Capsule().fill(Color.white.opacity(0.08)))

                        Spacer()

                        Text(timestampString)
                            .font(.system(size: 10))
                            .foregroundColor(.secondary.opacity(0.7))
                    }
                    .padding(.bottom, 2)

                    MarkdownMessage(text: message.content, isStreaming: isStreaming, onOpenCodeAssistant: onOpenCodeAssistant)

                    if isStreaming {
                        RoundedRectangle(cornerRadius: 2)
                            .fill(Color.accentColor)
                            .frame(width: 8, height: 14)
                            .opacity(0.8)
                    } else {
                        // Metrics & Action footer
                        HStack(spacing: 8) {
                            if let tps = message.tokensPerSecond, tps > 0 {
                                HStack(spacing: 4) {
                                    Image(systemName: "gauge.with.dots.needle.bottom.50percent")
                                        .font(.system(size: 9))
                                    Text(String(format: "%.1f tok/s", tps))
                                        .font(.system(size: 10, weight: .medium, design: .monospaced))
                                }
                                .foregroundColor(.secondary)
                                .padding(.horizontal, 6)
                                .padding(.vertical, 2)
                                .background(Capsule().fill(Color.white.opacity(0.06)))
                            }

                            if let dur = message.durationSeconds, dur > 0 {
                                HStack(spacing: 3) {
                                    Image(systemName: "clock")
                                        .font(.system(size: 9))
                                    Text(String(format: "%.1fs", dur))
                                        .font(.system(size: 10, design: .monospaced))
                                }
                                .foregroundColor(.secondary.opacity(0.7))
                            }

                            Spacer()

                            if message.variants.count > 1 {
                                HStack(spacing: 3) {
                                    Button {
                                        if message.selectedVariantIndex > 0 {
                                            store.selectVariant(messageId: message.id, index: message.selectedVariantIndex - 1)
                                        }
                                    } label: {
                                        Image(systemName: "chevron.left")
                                            .font(.system(size: 8, weight: .bold))
                                            .foregroundColor(message.selectedVariantIndex > 0 ? .primary : .secondary.opacity(0.3))
                                            .frame(width: 16, height: 16)
                                    }
                                    .buttonStyle(.plain)
                                    .disabled(message.selectedVariantIndex <= 0)

                                    Text("\(message.selectedVariantIndex + 1)/\(message.variants.count)")
                                        .font(.system(size: 9.5, weight: .medium, design: .monospaced))
                                        .foregroundColor(.secondary)

                                    Button {
                                        if message.selectedVariantIndex < message.variants.count - 1 {
                                            store.selectVariant(messageId: message.id, index: message.selectedVariantIndex + 1)
                                        }
                                    } label: {
                                        Image(systemName: "chevron.right")
                                            .font(.system(size: 8, weight: .bold))
                                            .foregroundColor(message.selectedVariantIndex < message.variants.count - 1 ? .primary : .secondary.opacity(0.3))
                                            .frame(width: 16, height: 16)
                                    }
                                    .buttonStyle(.plain)
                                    .disabled(message.selectedVariantIndex >= message.variants.count - 1)
                                }
                                .padding(.horizontal, 4)
                                .padding(.vertical, 2)
                                .background(Capsule().fill(Color.white.opacity(0.08)))
                            }

                            if isLastAssistant {
                                Menu {
                                    Button("Regenerate with \(store.friendlyName(for: store.selectedModel))") {
                                        LiquidGlass.haptic(.alignment)
                                        store.regenerateLastResponse()
                                    }
                                    if !store.models.isEmpty {
                                        Divider()
                                        ForEach(store.models, id: \.self) { m in
                                            Button("Regenerate with \(store.friendlyName(for: m))") {
                                                LiquidGlass.haptic(.alignment)
                                                store.regenerateLastResponse(withModel: m)
                                            }
                                        }
                                    }
                                } label: {
                                    HStack(spacing: 3) {
                                        Image(systemName: "arrow.clockwise")
                                            .font(.system(size: 9))
                                        Text("Regenerate")
                                            .font(.system(size: 10))
                                    }
                                    .foregroundColor(.secondary.opacity(0.85))
                                    .padding(.horizontal, 6)
                                    .padding(.vertical, 2)
                                    .background(Capsule().fill(Color.white.opacity(0.06)))
                                }
                                .menuStyle(.borderlessButton)
                                .fixedSize()
                                .help("Regenerate response from previous prompt")
                            }

                            Button {
                                NSPasteboard.general.clearContents()
                                NSPasteboard.general.setString(message.content, forType: .string)
                                LiquidGlass.haptic(.alignment)
                                withAnimation(LiquidGlass.spring) {
                                    copiedFull = true
                                }
                                DispatchQueue.main.asyncAfter(deadline: .now() + 1.8) {
                                    withAnimation(LiquidGlass.spring) {
                                        copiedFull = false
                                    }
                                }
                            } label: {
                                HStack(spacing: 3) {
                                    Image(systemName: copiedFull ? "checkmark" : "doc.on.doc")
                                        .font(.system(size: 9))
                                    Text(copiedFull ? "Copied" : "Copy Response")
                                        .font(.system(size: 10))
                                }
                                .foregroundColor(copiedFull ? .green : .secondary.opacity(0.8))
                                .padding(.horizontal, 6)
                                .padding(.vertical, 2)
                                .background(Capsule().fill(Color.white.opacity(0.06)))
                            }
                            .buttonStyle(.plain)

                            Button {
                                store.forkThread(at: message.id)
                            } label: {
                                HStack(spacing: 3) {
                                    Image(systemName: "tuningfork")
                                        .font(.system(size: 9))
                                    Text("Fork")
                                        .font(.system(size: 10))
                                }
                                .foregroundColor(.secondary.opacity(0.85))
                                .padding(.horizontal, 6)
                                .padding(.vertical, 2)
                                .background(Capsule().fill(Color.white.opacity(0.06)))
                            }
                            .buttonStyle(.plain)
                            .help("Fork thread into a new conversation from this response")
                        }
                        .padding(.top, 4)
                    }
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .onHover { isHovered = $0 }
        }
    }
}

// MARK: - Message markdown (fenced code blocks + inline prose + <think> reasoning)

public enum MsgSegment: Hashable {
    case prose(String)
    case code(language: String, code: String)
    case thought(String)
}

/// Split assistant text on <think> blocks and ``` fences. Unclosed fences (mid-stream)
/// render the remainder as code; pure so tests can pin it.
public func splitMessage(_ text: String) -> [MsgSegment] {
    var segs: [MsgSegment] = []
    var remainingText = text

    // Check for <think> reasoning tags (supports multiple blocks and streaming unclosed blocks)
    while let thinkStart = remainingText.range(of: "<think>", options: .caseInsensitive) {
        let beforeThink = String(remainingText[..<thinkStart.lowerBound])
        if !beforeThink.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            segs.append(contentsOf: parseProseAndCode(beforeThink))
        }

        let afterStart = remainingText[thinkStart.upperBound...]
        if let thinkEnd = afterStart.range(of: "</think>", options: .caseInsensitive) {
            let thought = String(afterStart[..<thinkEnd.lowerBound]).trimmingCharacters(in: .whitespacesAndNewlines)
            if !thought.isEmpty {
                segs.append(.thought(thought))
            }
            remainingText = String(afterStart[thinkEnd.upperBound...]).trimmingCharacters(in: .whitespacesAndNewlines)
        } else {
            // Unclosed think (mid-stream)
            let thought = String(afterStart).trimmingCharacters(in: .whitespacesAndNewlines)
            if !thought.isEmpty {
                segs.append(.thought(thought))
            }
            remainingText = ""
            break
        }
    }

    if !remainingText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
        segs.append(contentsOf: parseProseAndCode(remainingText))
    }

    return segs.isEmpty && !text.isEmpty ? [.prose(text)] : segs
}

private func parseProseAndCode(_ text: String) -> [MsgSegment] {
    var segs: [MsgSegment] = []
    var prose: [String] = []
    var code: [String]? = nil
    var lang = ""
    func flushProse() {
        let joined = prose.joined(separator: "\n")
        if !joined.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            segs.append(.prose(joined))
        }
        prose = []
    }
    for line in text.split(separator: "\n", omittingEmptySubsequences: false).map(String.init) {
        if code != nil, line.trimmingCharacters(in: .whitespaces).hasPrefix("```") {
            segs.append(.code(language: lang, code: (code ?? []).joined(separator: "\n")))
            code = nil
        } else if code != nil {
            code?.append(line)
        } else if line.trimmingCharacters(in: .whitespaces).hasPrefix("```") {
            flushProse()
            lang = String(line.trimmingCharacters(in: .whitespaces).dropFirst(3))
                .trimmingCharacters(in: .whitespaces)
            code = []
        } else {
            prose.append(line)
        }
    }
    if let c = code {
        segs.append(.code(language: lang, code: c.joined(separator: "\n")))
    }
    flushProse()
    return segs
}

public struct MarkdownMessage: View {
    public let text: String
    public var isStreaming: Bool
    public var onOpenCodeAssistant: ((String, String) -> Void)?

    public init(text: String, isStreaming: Bool = false, onOpenCodeAssistant: ((String, String) -> Void)? = nil) {
        self.text = text
        self.isStreaming = isStreaming
        self.onOpenCodeAssistant = onOpenCodeAssistant
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            ForEach(Array(splitMessage(text).enumerated()), id: \.offset) { _, seg in
                switch seg {
                case .prose(let p): ProseText(text: p)
                case .code(let l, let c): CodeBlock(language: l, code: c, onSendToCodeAssistant: onOpenCodeAssistant)
                case .thought(let th): ExecutiveThoughtBox(thought: th, isStreaming: isStreaming)
                }
            }
        }
    }
}

private struct ProseText: View {
    let text: String

    var body: some View {
        if let attr = try? AttributedString(
            markdown: text,
            options: AttributedString.MarkdownParsingOptions(interpretedSyntax: .inlineOnlyPreservingWhitespace)
        ) {
            Text(attr)
                .font(.system(size: 13.5))
                .lineSpacing(3)
                .textSelection(.enabled)
        } else {
            Text(text)
                .font(.system(size: 13.5))
                .lineSpacing(3)
                .textSelection(.enabled)
        }
    }
}

public struct CodeBlock: View {
    public let language: String
    public let code: String
    public var onSendToCodeAssistant: ((String, String) -> Void)?
    @State private var copied = false
    @State private var sentToAssistant = false

    public init(language: String, code: String, onSendToCodeAssistant: ((String, String) -> Void)? = nil) {
        self.language = language
        self.code = code
        self.onSendToCodeAssistant = onSendToCodeAssistant
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            // Claude / Codex Code Block Header Bar
            HStack(spacing: 8) {
                Text(language.isEmpty ? "code" : language)
                    .font(.system(size: 11, weight: .semibold, design: .monospaced))
                    .foregroundColor(.secondary)

                Spacer()

                if let onSendToCodeAssistant {
                    Button {
                        LiquidGlass.haptic(.alignment)
                        onSendToCodeAssistant(language, code)
                        withAnimation(LiquidGlass.spring) {
                            sentToAssistant = true
                        }
                        DispatchQueue.main.asyncAfter(deadline: .now() + 2.0) {
                            withAnimation(LiquidGlass.spring) {
                                sentToAssistant = false
                            }
                        }
                    } label: {
                        HStack(spacing: 4) {
                            Image(systemName: sentToAssistant ? "checkmark" : "curlybraces.square")
                                .font(.system(size: 10, weight: .semibold))
                                .foregroundColor(sentToAssistant ? .accentColor : .secondary)
                            Text(sentToAssistant ? "Opened" : "Send to Code Assistant")
                                .font(.system(size: 11, weight: .medium))
                                .foregroundColor(sentToAssistant ? .accentColor : .secondary)
                        }
                        .padding(.horizontal, 8)
                        .padding(.vertical, 4)
                        .background(Capsule().fill(Color.white.opacity(0.08)))
                    }
                    .buttonStyle(.plain)
                    .help("Open and edit this snippet in the Code Assistant workbench (⌘2)")
                }

                Button {
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString(code, forType: .string)
                    LiquidGlass.haptic(.alignment)
                    withAnimation(LiquidGlass.spring) {
                        copied = true
                    }
                    DispatchQueue.main.asyncAfter(deadline: .now() + 1.8) {
                        withAnimation(LiquidGlass.spring) {
                            copied = false
                        }
                    }
                } label: {
                    HStack(spacing: 4) {
                        Image(systemName: copied ? "checkmark" : "doc.on.doc")
                            .font(.system(size: 10, weight: .semibold))
                            .foregroundColor(copied ? .green : .secondary)
                        Text(copied ? "Copied" : "Copy")
                            .font(.system(size: 11, weight: .medium))
                            .foregroundColor(copied ? .green : .secondary)
                    }
                    .padding(.horizontal, 8)
                    .padding(.vertical, 4)
                    .background(Capsule().fill(Color.white.opacity(0.08)))
                }
                .buttonStyle(.plain)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 7)
            .background(Color.black.opacity(0.35))
            .overlay(Divider().opacity(0.25), alignment: .bottom)

            // Code Content
            ScrollView(.horizontal, showsIndicators: true) {
                Text(code)
                    .font(.system(size: 12.5, design: .monospaced))
                    .lineSpacing(2)
                    .textSelection(.enabled)
                    .padding(12)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .background(
            RoundedRectangle(cornerRadius: 10, style: .continuous)
                .fill(Color.black.opacity(0.45))
                .overlay(
                    RoundedRectangle(cornerRadius: 10, style: .continuous)
                        .strokeBorder(Color.white.opacity(0.12), lineWidth: 1)
                )
        )
        .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
    }
}

// MARK: - System Tuning Popover (Model Hyperparameters & AGENTS.md Standards)

struct SystemTuningPopoverView: View {
    @ObservedObject var store: ChatStore

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            // Title Header
            HStack(spacing: 8) {
                Image(systemName: "slider.horizontal.3")
                    .font(.system(size: 12, weight: .semibold))
                    .foregroundColor(.accentColor)
                VStack(alignment: .leading, spacing: 1) {
                    Text("Model Hyperparameters")
                        .font(.system(size: 12.5, weight: .bold))
                    Text("Apple Silicon runtime sampling (AGENTS.md)")
                        .font(.system(size: 10))
                        .foregroundColor(.secondary)
                }
                Spacer()
                Button("Reset") {
                    LiquidGlass.haptic(.alignment)
                    store.temperature = 0.6
                    store.topP = 0.95
                    store.contextCap = 32768
                    store.thinkingMode = false
                    store.customSystemPrompt = "You are LAC Assistant, a world-class local agentic AI running on Apple Silicon. You are concise, precise, memory-efficient, and generate clean, robust solutions."
                }
                .font(.system(size: 10))
                .buttonStyle(.plain)
                .foregroundColor(.accentColor)
            }
            .padding(.bottom, 2)

            Divider().opacity(0.3)

            // Temperature Slider
            VStack(alignment: .leading, spacing: 4) {
                HStack {
                    Text("Temperature")
                        .font(.system(size: 11, weight: .medium))
                    Spacer()
                    Text(String(format: "%.2f", store.temperature))
                        .font(.system(size: 11, weight: .bold, design: .monospaced))
                        .foregroundColor(store.temperature == 0.6 ? .accentColor : .primary)
                    if store.temperature == 0.6 {
                        Text("(MTP default)")
                            .font(.system(size: 9.5))
                            .foregroundColor(.secondary)
                    }
                }
                Slider(value: $store.temperature, in: 0.0...1.2, step: 0.05)
                    .tint(.accentColor)
            }

            // Top-P Slider
            VStack(alignment: .leading, spacing: 4) {
                HStack {
                    Text("Top-P Nucleus Sampling")
                        .font(.system(size: 11, weight: .medium))
                    Spacer()
                    Text(String(format: "%.2f", store.topP))
                        .font(.system(size: 11, weight: .bold, design: .monospaced))
                        .foregroundColor(store.topP == 0.95 ? .accentColor : .primary)
                    if store.topP == 0.95 {
                        Text("(default)")
                            .font(.system(size: 9.5))
                            .foregroundColor(.secondary)
                    }
                }
                Slider(value: $store.topP, in: 0.1...1.0, step: 0.05)
                    .tint(.accentColor)
            }

            // Context Cap Picker
            VStack(alignment: .leading, spacing: 6) {
                HStack {
                    Text("Context Window Cap")
                        .font(.system(size: 11, weight: .medium))
                    Spacer()
                    Text(contextCapLabel(store.contextCap))
                        .font(.system(size: 10.5, design: .monospaced))
                        .foregroundColor(.secondary)
                }

                Picker("Context Cap", selection: $store.contextCap) {
                    Text("16K Cap").tag(16384)
                    Text("32K Standard").tag(32768)
                    Text("64K Max").tag(65536)
                }
                .pickerStyle(.segmented)
            }

            // Thinking Mode Toggle
            HStack(alignment: .center) {
                VStack(alignment: .leading, spacing: 2) {
                    HStack(spacing: 5) {
                        Image(systemName: "brain.head.profile")
                            .font(.system(size: 11))
                            .foregroundColor(store.thinkingMode ? .accentColor : .secondary)
                        Text("Thinking Mode (<think>)")
                            .font(.system(size: 11, weight: .semibold))
                    }
                    Text("Enable deep step-by-step chain-of-thought analysis")
                        .font(.system(size: 9.5))
                        .foregroundColor(.secondary)
                }
                Spacer()
                Toggle("", isOn: $store.thinkingMode)
                    .toggleStyle(.switch)
                    .labelsHidden()
            }
            .padding(.vertical, 2)

            Divider().opacity(0.3)

            // System Prompt Presets
            VStack(alignment: .leading, spacing: 6) {
                Text("System Persona Presets")
                    .font(.system(size: 10.5, weight: .semibold))
                    .foregroundColor(.secondary)

                HStack(spacing: 6) {
                    presetButton("LAC Systems", prompt: "You are LAC Assistant, a world-class local agentic AI running on Apple Silicon. You are concise, precise, memory-efficient, and generate clean, robust solutions.")
                    presetButton("Minimalist", prompt: "You are an expert systems engineer. Output only exact code, unit tests, and surgical diffs with zero conversational fluff.")
                    presetButton("Auditor", prompt: "You are an adversarial systems auditor. Inspect for race conditions, memory leaks, panics, buffer overflows, and socket stalls.")
                }
            }
        }
        .padding(16)
        .frame(width: 350)
        .background(.ultraThinMaterial)
    }

    private func contextCapLabel(_ cap: Int) -> String {
        switch cap {
        case 16384: return "16,384 tokens (fast)"
        case 32768: return "32,768 tokens (standard)"
        case 65536: return "65,536 tokens (deep)"
        default: return "\(cap) tokens"
        }
    }

    private func presetButton(_ title: String, prompt: String) -> some View {
        Button {
            LiquidGlass.haptic(.alignment)
            store.customSystemPrompt = prompt
        } label: {
            Text(title)
                .font(.system(size: 10, weight: .medium))
                .padding(.horizontal, 8)
                .padding(.vertical, 4)
                .background(
                    Capsule()
                        .fill(store.customSystemPrompt == prompt ? Color.accentColor.opacity(0.20) : Color.white.opacity(0.06))
                        .overlay(Capsule().strokeBorder(store.customSystemPrompt == prompt ? Color.accentColor.opacity(0.4) : Color.white.opacity(0.1), lineWidth: 0.5))
                )
        }
        .buttonStyle(.plain)
    }
}
