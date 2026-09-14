import AppKit
import SwiftUI

// MARK: - Navigation destination

public enum AppNavigationSection: Hashable {
    case chat
    case codeAssistant
    case dashboard
    case modelHub
    case loops
}

// MARK: - Claude Desktop / Codex Main App Layout

public struct MainAppView: View {
    @EnvironmentObject private var network: NetworkManager
    @EnvironmentObject private var chatStore: ChatStore
    @StateObject private var codeStore = CodeAssistantStore()
    @StateObject private var loopsStore = LoopsStore()
    @State private var navigationSection: AppNavigationSection = .chat
    @State private var sidebarVisibility: NavigationSplitViewVisibility = .all
    @State private var isCommandPalettePresented = false

    public init() {}

    public var body: some View {
        NavigationSplitView(columnVisibility: $sidebarVisibility) {
            SidebarView(
                selection: $navigationSection,
                isCommandPalettePresented: $isCommandPalettePresented
            )
                .navigationSplitViewColumnWidth(min: 250, ideal: 275, max: 320)
        } detail: {
            Group {
                switch navigationSection {
                case .chat:
                    ChatView(
                        sidebarVisibility: $sidebarVisibility,
                        onOpenDashboard: {
                            withAnimation(LiquidGlass.spring) {
                                navigationSection = .dashboard
                            }
                        },
                        onOpenModelHub: {
                            withAnimation(LiquidGlass.spring) {
                                navigationSection = .modelHub
                            }
                        },
                        onOpenCodeAssistant: { lang, code in
                            codeStore.selectedLanguage = lang
                            codeStore.sourceCode = code
                            withAnimation(LiquidGlass.spring) {
                                navigationSection = .codeAssistant
                            }
                        }
                    )
                case .codeAssistant:
                    CodeAssistantView(
                        store: codeStore,
                        sidebarVisibility: $sidebarVisibility
                    )
                case .dashboard:
                    VStack(spacing: 0) {
                        // Top bar on Dashboard to quickly return to Chat
                        HStack {
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

                            Button {
                                withAnimation(LiquidGlass.spring) {
                                    navigationSection = .chat
                                }
                            } label: {
                                HStack(spacing: 6) {
                                    Image(systemName: "chevron.left")
                                        .font(.system(size: 11, weight: .semibold))
                                    Text("Chat")
                                        .font(.system(size: 12, weight: .medium))
                                }
                            }
                            .lacGlass()

                            Spacer()

                            LiquidGlassBadge(
                                title: "Ops Mode",
                                icon: "gauge",
                                color: .blue
                            )
                        }
                        .padding(.horizontal, 16)
                        .padding(.vertical, 8)
                        .background(.ultraThinMaterial)
                        .overlay(Divider().opacity(0.3), alignment: .bottom)

                        DashboardView()
                    }
                case .modelHub:
                    ModelHubView(
                        sidebarVisibility: $sidebarVisibility,
                        onSelectModel: { modelId in
                            chatStore.selectedModel = modelId
                            withAnimation(LiquidGlass.spring) {
                                navigationSection = .chat
                            }
                        }
                    )
                case .loops:
                    LoopsView(
                        store: loopsStore,
                        sidebarVisibility: $sidebarVisibility
                    )
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .overlay {
            if isCommandPalettePresented {
                CommandPaletteView(
                    isPresented: $isCommandPalettePresented,
                    network: network,
                    chatStore: chatStore,
                    navigationSection: $navigationSection,
                    sidebarVisibility: $sidebarVisibility
                )
                .transition(.opacity.combined(with: .scale(scale: 0.98)))
            }
        }
        .animation(LiquidGlass.spring, value: isCommandPalettePresented)
        .background {
            Group {
                Button("") {
                    withAnimation(LiquidGlass.spring) { isCommandPalettePresented.toggle() }
                }
                .keyboardShortcut("k", modifiers: .command)

                Button("") {
                    withAnimation(LiquidGlass.spring) { navigationSection = .chat }
                }
                .keyboardShortcut("1", modifiers: .command)

                Button("") {
                    withAnimation(LiquidGlass.spring) { navigationSection = .codeAssistant }
                }
                .keyboardShortcut("2", modifiers: .command)

                Button("") {
                    withAnimation(LiquidGlass.spring) { navigationSection = .dashboard }
                }
                .keyboardShortcut("3", modifiers: .command)

                Button("") {
                    withAnimation(LiquidGlass.spring) { navigationSection = .modelHub }
                }
                .keyboardShortcut("4", modifiers: .command)

                Button("") {
                    withAnimation(LiquidGlass.spring) { navigationSection = .loops }
                }
                .keyboardShortcut("5", modifiers: .command)
            }
            .opacity(0)
            .allowsHitTesting(false)
        }
    }
}

// MARK: - Sidebar View (Claude / Codex pattern)

struct SidebarView: View {
    @Binding var selection: AppNavigationSection
    @Binding var isCommandPalettePresented: Bool
    @EnvironmentObject private var network: NetworkManager
    @EnvironmentObject private var chatStore: ChatStore
    @State private var hoveredThreadId: String?
    @State private var renamingThreadId: String?
    @State private var renameText: String = ""
    @State private var threadSearchText: String = ""

    var body: some View {
        VStack(spacing: 0) {
            brandHeader
                .padding(.horizontal, 14)
                .padding(.top, 14)
                .padding(.bottom, 10)

            newChatButton
                .padding(.horizontal, 14)
                .padding(.bottom, 8)

            // Real-time thread search bar
            HStack(spacing: 6) {
                Image(systemName: "magnifyingglass")
                    .font(.system(size: 11))
                    .foregroundColor(.secondary)
                TextField("Search chats…", text: $threadSearchText)
                    .textFieldStyle(.plain)
                    .font(.system(size: 11.5))
                if !threadSearchText.isEmpty {
                    Button {
                        threadSearchText = ""
                    } label: {
                        Image(systemName: "xmark.circle.fill")
                            .font(.system(size: 10))
                            .foregroundColor(.secondary)
                    }
                    .buttonStyle(.plain)
                }
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 5)
            .background(
                RoundedRectangle(cornerRadius: 7, style: .continuous)
                    .fill(Color.white.opacity(0.06))
            )
            .padding(.horizontal, 14)
            .padding(.bottom, 10)

            Divider()
                .opacity(0.3)
                .padding(.horizontal, 14)
                .padding(.bottom, 8)

            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    workspaceSection
                    recentsSection
                }
                .padding(.horizontal, 10)
                .padding(.vertical, 4)
            }

            Spacer(minLength: 8)

            Divider()
                .opacity(0.3)
                .padding(.horizontal, 14)

            telemetryFooterCard
                .padding(12)
        }
        .background(.ultraThinMaterial)
    }

    // MARK: Brand & Status Header

    private var brandHeader: some View {
        HStack(spacing: 10) {
            LACAvatarView(diameter: 28, isPulsing: network.response != nil)

            VStack(alignment: .leading, spacing: 2) {
                Text("Loop LAC Studio")
                    .font(.system(size: 13, weight: .bold, design: .rounded))
                    .foregroundColor(.primary)

                HStack(spacing: 5) {
                    Circle()
                        .fill(network.response != nil ? Color.green : Color.red)
                        .frame(width: 6, height: 6)
                    Text("Gateway :\(network.port)")
                        .font(.system(size: 10, weight: .medium))
                        .foregroundColor(.secondary)
                }
            }

            Spacer()

            Button {
                LiquidGlass.haptic(.alignment)
                withAnimation(LiquidGlass.spring) {
                    isCommandPalettePresented.toggle()
                }
            } label: {
                HStack(spacing: 2) {
                    Image(systemName: "command")
                        .font(.system(size: 8, weight: .bold))
                    Text("K")
                        .font(.system(size: 9, weight: .bold))
                }
                .padding(.horizontal, 5)
                .padding(.vertical, 3)
                .background(
                    RoundedRectangle(cornerRadius: 5, style: .continuous)
                        .fill(Color.white.opacity(0.08))
                )
                .foregroundColor(.secondary)
            }
            .buttonStyle(.plain)
            .help("Command Palette (⌘K)")

            Button {
                LiquidGlass.haptic(.alignment)
                network.fetch()
                Task { await chatStore.fetchModels() }
            } label: {
                Image(systemName: "arrow.clockwise")
                    .font(.system(size: 11))
                    .foregroundColor(.secondary)
                    .frame(width: 22, height: 22)
            }
            .buttonStyle(.plain)
            .help("Refresh gateway and models")
        }
    }

    // MARK: Claude-style New Chat Button

    private var newChatButton: some View {
        Button {
            LiquidGlass.haptic(.alignment)
            _ = chatStore.newThread()
            withAnimation(LiquidGlass.spring) {
                selection = .chat
            }
        } label: {
            HStack(spacing: 8) {
                Image(systemName: "plus")
                    .font(.system(size: 13, weight: .semibold))
                Text("New Chat")
                    .font(.system(size: 13, weight: .semibold))
                Spacer()
                Text("⌘N")
                    .font(.system(size: 11, weight: .medium, design: .monospaced))
                    .foregroundColor(.secondary.opacity(0.8))
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            .background(
                RoundedRectangle(cornerRadius: 10, style: .continuous)
                    .fill(Color.accentColor.opacity(0.14))
                    .overlay(LiquidGlass.specularBorder(cornerRadius: 10, isHovered: false))
            )
            .foregroundColor(.primary)
        }
        .buttonStyle(.plain)
        .keyboardShortcut("n", modifiers: .command)
    }

    // MARK: Workspace Section

    private var workspaceSection: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text("WORKSPACE")
                .font(.system(size: 10, weight: .bold))
                .foregroundColor(.secondary.opacity(0.8))
                .padding(.horizontal, 8)
                .padding(.bottom, 2)

            // Chat option
            Button {
                withAnimation(LiquidGlass.spring) {
                    selection = .chat
                }
            } label: {
                HStack(spacing: 8) {
                    Image(systemName: "bubble.left.and.bubble.right.fill")
                        .font(.system(size: 12))
                        .foregroundColor(selection == .chat ? .accentColor : .secondary)
                        .frame(width: 18)
                    Text("Chat Assistant")
                        .font(.system(size: 12, weight: selection == .chat ? .semibold : .regular))
                    Spacer()
                    if !chatStore.threads.isEmpty {
                        Text("\(chatStore.threads.count)")
                            .font(.system(size: 10, weight: .medium))
                            .foregroundColor(.secondary)
                            .padding(.horizontal, 6)
                            .padding(.vertical, 2)
                            .background(Capsule().fill(Color.white.opacity(0.08)))
                    }
                }
                .padding(.horizontal, 8)
                .padding(.vertical, 6)
                .background(
                    RoundedRectangle(cornerRadius: 8, style: .continuous)
                        .fill(selection == .chat ? Color.white.opacity(0.10) : Color.clear)
                )
                .foregroundColor(.primary)
            }
            .buttonStyle(.plain)

            // Code Assistant option
            Button {
                withAnimation(LiquidGlass.spring) {
                    selection = .codeAssistant
                }
            } label: {
                HStack(spacing: 8) {
                    Image(systemName: "curlybraces.square.fill")
                        .font(.system(size: 12))
                        .foregroundColor(selection == .codeAssistant ? .accentColor : .secondary)
                        .frame(width: 18)
                    Text("Code Assistant")
                        .font(.system(size: 12, weight: selection == .codeAssistant ? .semibold : .regular))
                    Spacer()
                    Text("AI")
                        .font(.system(size: 9, weight: .bold, design: .monospaced))
                        .padding(.horizontal, 5)
                        .padding(.vertical, 1)
                        .background(Capsule().fill(Color.purple.opacity(0.2)))
                        .foregroundColor(.purple)
                }
                .padding(.horizontal, 8)
                .padding(.vertical, 6)
                .background(
                    RoundedRectangle(cornerRadius: 8, style: .continuous)
                        .fill(selection == .codeAssistant ? Color.white.opacity(0.10) : Color.clear)
                )
                .foregroundColor(.primary)
            }
            .buttonStyle(.plain)

            // Ops Dashboard option
            Button {
                withAnimation(LiquidGlass.spring) {
                    selection = .dashboard
                }
            } label: {
                HStack(spacing: 8) {
                    Image(systemName: "gauge.with.dots.needle.bottom.50percent")
                        .font(.system(size: 12))
                        .foregroundColor(selection == .dashboard ? .accentColor : .secondary)
                        .frame(width: 18)
                    Text("Ops Dashboard")
                        .font(.system(size: 12, weight: selection == .dashboard ? .semibold : .regular))
                    Spacer()
                    Circle()
                        .fill(network.response != nil ? Color.green : Color.red)
                        .frame(width: 6, height: 6)
                }
                .padding(.horizontal, 8)
                .padding(.vertical, 6)
                .background(
                    RoundedRectangle(cornerRadius: 8, style: .continuous)
                        .fill(selection == .dashboard ? Color.white.opacity(0.10) : Color.clear)
                )
                .foregroundColor(.primary)
            }
            .buttonStyle(.plain)

            // Model Hub (Hugging Face / LM Studio style)
            Button {
                withAnimation(LiquidGlass.spring) {
                    selection = .modelHub
                }
            } label: {
                HStack(spacing: 8) {
                    Image(systemName: "square.grid.2x2.fill")
                        .font(.system(size: 12))
                        .foregroundColor(selection == .modelHub ? .accentColor : .secondary)
                        .frame(width: 18)
                    Text("Model Hub")
                        .font(.system(size: 12, weight: selection == .modelHub ? .semibold : .regular))
                    Spacer()
                    Text("HF")
                        .font(.system(size: 9, weight: .bold, design: .monospaced))
                        .padding(.horizontal, 5)
                        .padding(.vertical, 1)
                        .background(Capsule().fill(Color.orange.opacity(0.2)))
                        .foregroundColor(.orange)
                }
                .padding(.horizontal, 8)
                .padding(.vertical, 6)
                .background(
                    RoundedRectangle(cornerRadius: 8, style: .continuous)
                        .fill(selection == .modelHub ? Color.white.opacity(0.10) : Color.clear)
                )
                .foregroundColor(.primary)
            }
            .buttonStyle(.plain)

            // Autonomous Loops (Kanban & multi-phase agents)
            Button {
                withAnimation(LiquidGlass.spring) {
                    selection = .loops
                }
            } label: {
                HStack(spacing: 8) {
                    Image(systemName: "arrow.triangle.2.circlepath.circle.fill")
                        .font(.system(size: 12))
                        .foregroundColor(selection == .loops ? .accentColor : .secondary)
                        .frame(width: 18)
                    Text("Autonomous Loops")
                        .font(.system(size: 12, weight: selection == .loops ? .semibold : .regular))
                    Spacer()
                    Text("LOOP")
                        .font(.system(size: 9, weight: .bold, design: .monospaced))
                        .padding(.horizontal, 5)
                        .padding(.vertical, 1)
                        .background(Capsule().fill(Color.teal.opacity(0.2)))
                        .foregroundColor(.teal)
                }
                .padding(.horizontal, 8)
                .padding(.vertical, 6)
                .background(
                    RoundedRectangle(cornerRadius: 8, style: .continuous)
                        .fill(selection == .loops ? Color.white.opacity(0.10) : Color.clear)
                )
                .foregroundColor(.primary)
            }
            .buttonStyle(.plain)
        }
    }

    enum ThreadPeriod: String, CaseIterable {
        case today = "Today"
        case yesterday = "Yesterday"
        case previous7Days = "Previous 7 Days"
        case older = "Older"
    }

    private func periodFor(thread: ChatThread) -> ThreadPeriod {
        guard let last = thread.messages.last else { return .today }
        let date = Date(timeIntervalSince1970: TimeInterval(last.ts))
        let cal = Calendar.current
        if cal.isDateInToday(date) { return .today }
        if cal.isDateInYesterday(date) { return .yesterday }
        if let weekAgo = cal.date(byAdding: .day, value: -7, to: Date()), date >= weekAgo {
            return .previous7Days
        }
        return .older
    }

    private var filteredThreads: [ChatThread] {
        let q = threadSearchText.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        if q.isEmpty { return chatStore.threads }
        return chatStore.threads.filter { t in
            t.title.lowercased().contains(q) || t.messages.contains { $0.content.lowercased().contains(q) }
        }
    }

    // MARK: Recents Thread List (Claude / Codex pattern)

    private var recentsSection: some View {
        VStack(alignment: .leading, spacing: 10) {
            if chatStore.threads.isEmpty {
                Text("No recent conversations")
                    .font(.system(size: 11))
                    .foregroundColor(.secondary.opacity(0.6))
                    .padding(.horizontal, 8)
                    .padding(.vertical, 8)
            } else if !threadSearchText.isEmpty {
                VStack(alignment: .leading, spacing: 4) {
                    Text("SEARCH RESULTS (\(filteredThreads.count))")
                        .font(.system(size: 9.5, weight: .bold))
                        .foregroundColor(.secondary.opacity(0.8))
                        .padding(.horizontal, 8)
                    if filteredThreads.isEmpty {
                        Text("No matching conversations")
                            .font(.system(size: 11))
                            .foregroundColor(.secondary.opacity(0.6))
                            .padding(.horizontal, 8)
                            .padding(.vertical, 4)
                    } else {
                        ForEach(filteredThreads) { thread in
                            threadRow(thread: thread)
                        }
                    }
                }
            } else {
                ForEach(ThreadPeriod.allCases, id: \.self) { period in
                    let periodThreads = chatStore.threads.filter { periodFor(thread: $0) == period }
                    if !periodThreads.isEmpty {
                        VStack(alignment: .leading, spacing: 3) {
                            Text(period.rawValue.uppercased())
                                .font(.system(size: 9.5, weight: .bold))
                                .foregroundColor(.secondary.opacity(0.8))
                                .padding(.horizontal, 8)
                                .padding(.top, 4)
                            ForEach(periodThreads) { thread in
                                threadRow(thread: thread)
                            }
                        }
                    }
                }
            }
        }
    }

    private func threadRow(thread: ChatThread) -> some View {
        let isSelected = selection == .chat && chatStore.activeThreadId == thread.id
        let isHovered = hoveredThreadId == thread.id

        return HStack(spacing: 8) {
            Image(systemName: "message")
                .font(.system(size: 11))
                .foregroundColor(isSelected ? .accentColor : .secondary)
                .frame(width: 14)

            if renamingThreadId == thread.id {
                TextField("Thread title", text: $renameText, onCommit: {
                    chatStore.renameThread(thread.id, title: renameText)
                    renamingThreadId = nil
                })
                .textFieldStyle(.plain)
                .font(.system(size: 12))
                .padding(.horizontal, 4)
                .background(Color.black.opacity(0.3))
            } else {
                Text(thread.title)
                    .font(.system(size: 12, weight: isSelected ? .semibold : .regular))
                    .lineLimit(1)
                    .foregroundColor(.primary)
            }

            Spacer()

            if isHovered && renamingThreadId != thread.id {
                Button {
                    LiquidGlass.haptic(.alignment)
                    withAnimation(LiquidGlass.spring) {
                        chatStore.deleteThread(thread.id)
                    }
                } label: {
                    Image(systemName: "trash")
                        .font(.system(size: 10))
                        .foregroundColor(.secondary.opacity(0.8))
                        .padding(4)
                }
                .buttonStyle(.plain)
                .help("Delete conversation")
            } else if !thread.messages.isEmpty {
                Text(thread.relativeTime)
                    .font(.system(size: 9.5))
                    .foregroundColor(.secondary.opacity(0.6))
            }
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 6)
        .background(
            RoundedRectangle(cornerRadius: 8, style: .continuous)
                .fill(isSelected ? Color.white.opacity(0.12) : (isHovered ? Color.white.opacity(0.05) : Color.clear))
                .overlay(
                    RoundedRectangle(cornerRadius: 8, style: .continuous)
                        .strokeBorder(isSelected ? Color.white.opacity(0.18) : Color.clear, lineWidth: 1)
                )
        )
        .contentShape(Rectangle())
        .onTapGesture {
            chatStore.select(thread.id)
            withAnimation(LiquidGlass.spring) {
                selection = .chat
            }
        }
        .onHover { hovering in
            hoveredThreadId = hovering ? thread.id : nil
        }
        .contextMenu {
            Button {
                renameText = thread.title
                renamingThreadId = thread.id
            } label: {
                Label("Rename", systemImage: "pencil")
            }

            Button(role: .destructive) {
                withAnimation(LiquidGlass.spring) {
                    chatStore.deleteThread(thread.id)
                }
            } label: {
                Label("Delete", systemImage: "trash")
            }
        }
    }

    // MARK: Telemetry Footer Card (Codex/Claude style)

    private var telemetryFooterCard: some View {
        Button {
            withAnimation(LiquidGlass.spring) {
                selection = .dashboard
            }
        } label: {
            VStack(alignment: .leading, spacing: 6) {
                HStack {
                    Circle()
                        .fill(network.response != nil ? Color.green : Color.red)
                        .frame(width: 7, height: 7)
                    Text("Gateway :\(network.port)")
                        .font(.system(size: 11, weight: .semibold))
                    Spacer()
                    Text(network.response?.preferred.uppercased() ?? "LOCAL")
                        .font(.system(size: 9, weight: .bold))
                        .padding(.horizontal, 5)
                        .padding(.vertical, 1)
                        .background(Capsule().fill(Color.accentColor.opacity(0.2)))
                        .foregroundColor(.accentColor)
                }

                if let host = network.host {
                    HStack(spacing: 8) {
                        if let free = host.free_ram_gib {
                            Text(String(format: "%.1f GB free", free))
                                .font(.system(size: 10, design: .monospaced))
                                .foregroundColor(.secondary)
                            Text("•")
                                .font(.system(size: 10))
                                .foregroundColor(.secondary.opacity(0.4))
                        }
                        let thermal = host.thermal ?? "nominal"
                        Text(thermal.capitalized)
                            .font(.system(size: 10))
                            .foregroundColor(thermal.lowercased() == "nominal" ? .green : .orange)
                    }
                } else {
                    Text("Apple Silicon • Private Gateway")
                        .font(.system(size: 10))
                        .foregroundColor(.secondary)
                }
            }
            .padding(10)
            .liquidGlassCard(cornerRadius: 10, hoverable: true)
        }
        .buttonStyle(.plain)
        .help("Click to view full Ops Dashboard")
    }
}
