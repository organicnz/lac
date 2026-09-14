import AppKit
import SwiftUI

// MARK: - Command Palette Item

public struct PaletteItem: Identifiable, Hashable {
    public let id = UUID()
    public let title: String
    public let subtitle: String?
    public let icon: String
    public let category: String
    public let shortcut: String?
    public let action: () -> Void

    public func hash(into hasher: inout Hasher) {
        hasher.combine(id)
    }

    public static func == (lhs: PaletteItem, rhs: PaletteItem) -> Bool {
        lhs.id == rhs.id
    }
}

// MARK: - Command Palette View (Spotlight / ⌘K)

public struct CommandPaletteView: View {
    @Binding var isPresented: Bool
    @ObservedObject var network: NetworkManager
    @ObservedObject var chatStore: ChatStore
    @ObservedObject var codeStore: CodeAssistantStore
    @Binding var navigationSection: AppNavigationSection
    @Binding var sidebarVisibility: NavigationSplitViewVisibility

    @State private var query = ""
    @State private var selectedIndex = 0

    init(
        isPresented: Binding<Bool>,
        network: NetworkManager,
        chatStore: ChatStore,
        codeStore: CodeAssistantStore,
        navigationSection: Binding<AppNavigationSection>,
        sidebarVisibility: Binding<NavigationSplitViewVisibility>
    ) {
        self._isPresented = isPresented
        self.network = network
        self.chatStore = chatStore
        self.codeStore = codeStore
        self._navigationSection = navigationSection
        self._sidebarVisibility = sidebarVisibility
    }

    private var allItems: [PaletteItem] {
        var items: [PaletteItem] = []

        // 1. Workspaces
        items.append(PaletteItem(
            title: "Chat Assistant",
            subtitle: "Agentic conversation canvas",
            icon: "bubble.left.and.bubble.right.fill",
            category: "Workspaces",
            shortcut: "⌘1",
            action: { navigationSection = .chat }
        ))
        items.append(PaletteItem(
            title: "Code Assistant",
            subtitle: "Split code canvas, refactoring & tests",
            icon: "curlybraces.square.fill",
            category: "Workspaces",
            shortcut: "⌘2",
            action: { navigationSection = .codeAssistant }
        ))
        items.append(PaletteItem(
            title: "Ops Dashboard",
            subtitle: "Real-time ports, memory & thermals",
            icon: "gauge.with.dots.needle.bottom.50percent",
            category: "Workspaces",
            shortcut: "⌘3",
            action: { navigationSection = .dashboard }
        ))
        items.append(PaletteItem(
            title: "Model Hub",
            subtitle: "Browse Hugging Face MLX & GGUF models",
            icon: "square.grid.2x2.fill",
            category: "Workspaces",
            shortcut: "⌘4",
            action: { navigationSection = .modelHub }
        ))
        items.append(PaletteItem(
            title: "Autonomous Loops",
            subtitle: "Kanban task pipeline & multi-phase agents",
            icon: "arrow.triangle.2.circlepath.circle.fill",
            category: "Workspaces",
            shortcut: "⌘5",
            action: { navigationSection = .loops }
        ))

        // 2. Chat Actions
        items.append(PaletteItem(
            title: "New Chat",
            subtitle: "Create a fresh conversation thread",
            icon: "plus.circle.fill",
            category: "Actions",
            shortcut: "⌘N",
            action: {
                _ = chatStore.newThread()
                navigationSection = .chat
            }
        ))
        items.append(PaletteItem(
            title: "Toggle Sidebar",
            subtitle: "Expand or collapse navigation split view",
            icon: "sidebar.leading",
            category: "Actions",
            shortcut: "⌘B",
            action: {
                sidebarVisibility = (sidebarVisibility == .detailOnly) ? .all : .detailOnly
            }
        ))
        items.append(PaletteItem(
            title: "Quick Open File",
            subtitle: "Spotlight file search across workspace",
            icon: "doc.text.magnifyingglass",
            category: "Actions",
            shortcut: "⌘P",
            action: {
                navigationSection = .codeAssistant
            }
        ))
        items.append(PaletteItem(
            title: "Clear Active Conversation",
            subtitle: "Remove messages from current thread",
            icon: "arrow.counterclockwise",
            category: "Actions",
            shortcut: nil,
            action: { chatStore.clearActiveThread() }
        ))
        items.append(PaletteItem(
            title: "Run Cargo Tests",
            subtitle: "Execute cargo test on rust-src pure Rust backend",
            icon: "gearshape.2.fill",
            category: "Actions",
            shortcut: nil,
            action: {
                navigationSection = .codeAssistant
                codeStore.pendingConsoleCommand = .cargoTests
            }
        ))
        items.append(PaletteItem(
            title: "Run Swift Tests",
            subtitle: "Execute swift test on Loop LAC Studio suite",
            icon: "swift",
            category: "Actions",
            shortcut: nil,
            action: {
                navigationSection = .codeAssistant
                codeStore.pendingConsoleCommand = .swiftTests
            }
        ))
        items.append(PaletteItem(
            title: "Inspect Git Diff",
            subtitle: "Review uncommitted modifications in repository",
            icon: "arrow.triangle.branch",
            category: "Actions",
            shortcut: nil,
            action: {
                navigationSection = .codeAssistant
                codeStore.pendingConsoleCommand = .gitDiff
            }
        ))

        // 3. Gateway & Backend Controls
        items.append(PaletteItem(
            title: "Start LAC Router Gateway (:8000)",
            subtitle: "Launch background router daemon",
            icon: "bolt.fill",
            category: "Gateway",
            shortcut: nil,
            action: { Task { await network.startRouter() } }
        ))
        items.append(PaletteItem(
            title: "Start MLX Backend (Qwen 3.8)",
            subtitle: "Native Apple Silicon speed lane (:8080)",
            icon: "play.fill",
            category: "Gateway",
            shortcut: nil,
            action: { Task { await network.startMLX() } }
        ))
        items.append(PaletteItem(
            title: "Start llama-server (GGUF)",
            subtitle: "High precision quantized backend (:8081)",
            icon: "play.circle",
            category: "Gateway",
            shortcut: nil,
            action: { Task { await network.startLlama() } }
        ))
        items.append(PaletteItem(
            title: "Switch Gateway Backend -> Auto",
            subtitle: "Dynamic policy-ordered fallback",
            icon: "arrow.triangle.swap",
            category: "Gateway",
            shortcut: nil,
            action: { Task { await network.switchBackend("auto") } }
        ))
        items.append(PaletteItem(
            title: "Switch Gateway Backend -> Fastest",
            subtitle: "Lowest EWMA latency routing",
            icon: "speedometer",
            category: "Gateway",
            shortcut: nil,
            action: { Task { await network.switchBackend("fastest") } }
        ))

        // 4. Detected Local Models
        let modelList = chatStore.models.isEmpty ?
            ["mlx-community/Qwen3.8-27B-4bit", "Qwen/Qwen2.5-Coder-32B-Instruct"] : chatStore.models

        for m in modelList {
            items.append(PaletteItem(
                title: "Use Model: \(m)",
                subtitle: "Set active inference model",
                icon: "sparkles",
                category: "Models",
                shortcut: nil,
                action: {
                    chatStore.selectedModel = m
                    navigationSection = .chat
                }
            ))
        }

        // 5. Recent Conversations
        for t in chatStore.threads {
            items.append(PaletteItem(
                title: t.title,
                subtitle: "Conversation · \(t.relativeTime)",
                icon: "message",
                category: "Recent Chats",
                shortcut: nil,
                action: {
                    chatStore.select(t.id)
                    navigationSection = .chat
                }
            ))
        }

        return items
    }

    private var filteredItems: [PaletteItem] {
        let q = query.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        if q.isEmpty { return allItems }
        return allItems.filter {
            $0.title.lowercased().contains(q) ||
            ($0.subtitle?.lowercased().contains(q) ?? false) ||
            $0.category.lowercased().contains(q)
        }
    }

    public var body: some View {
        ZStack {
            // Darkened translucent blur scrim
            Color.black.opacity(0.45)
                .ignoresSafeArea()
                .onTapGesture {
                    dismiss()
                }

            // Spotlight Floating Elevated Modal
            VStack(spacing: 0) {
                // Search Input Header
                HStack(spacing: 12) {
                    Image(systemName: "magnifyingglass")
                        .font(.system(size: 15, weight: .medium))
                        .foregroundColor(.secondary)

                    TextField("Type a command, search chats, or switch models…", text: $query)
                        .textFieldStyle(.plain)
                        .font(.system(size: 14.5))
                        .onSubmit {
                            executeSelected()
                        }

                    if !query.isEmpty {
                        Button {
                            query = ""
                        } label: {
                            Image(systemName: "xmark.circle.fill")
                                .foregroundColor(.secondary)
                                .font(.system(size: 13))
                        }
                        .buttonStyle(.plain)
                    }

                    Text("ESC to close")
                        .font(.system(size: 10, weight: .semibold, design: .monospaced))
                        .padding(.horizontal, 6)
                        .padding(.vertical, 2)
                        .background(Capsule().fill(Color.white.opacity(0.08)))
                        .foregroundColor(.secondary)
                }
                .padding(.horizontal, 18)
                .padding(.vertical, 14)
                .background(.ultraThinMaterial)

                Divider().opacity(0.3)

                // Results list
                ScrollViewReader { proxy in
                    ScrollView {
                        VStack(alignment: .leading, spacing: 4) {                            if filteredItems.isEmpty {
                                HStack {
                                    Spacer()
                                    VStack(spacing: 8) {
                                        Text("No matching commands")
                                            .font(.system(size: 13, weight: .medium))
                                            .foregroundColor(.secondary)
                                        Text("Try searching for 'chat', 'code', 'model', or 'mlx'")
                                            .font(.system(size: 11))
                                            .foregroundColor(.secondary.opacity(0.6))
                                    }
                                    .padding(.vertical, 32)
                                    Spacer()
                                }
                            } else {
                                ForEach(Array(filteredItems.enumerated()), id: \.element.id) { index, item in
                                    let isSelected = index == selectedIndex
                                    itemRow(item: item, isSelected: isSelected)
                                        .id(index)
                                        .onTapGesture {
                                            execute(item)
                                        }
                                }
                            }
                        }
                        .padding(10)
                    }
                    .frame(maxHeight: 340)
                    .onChange(of: selectedIndex) { idx in
                        proxy.scrollTo(idx, anchor: .center)
                    }
                }

                Divider().opacity(0.2)

                // Footer hints
                HStack(spacing: 16) {
                    HStack(spacing: 4) {
                        Text("↑↓")
                            .font(.system(size: 10, weight: .bold, design: .monospaced))
                            .padding(.horizontal, 4)
                            .padding(.vertical, 1)
                            .background(RoundedRectangle(cornerRadius: 3).fill(Color.white.opacity(0.1)))
                        Text("Navigate")
                            .font(.system(size: 10.5))
                            .foregroundColor(.secondary)
                    }
                    HStack(spacing: 4) {
                        Text("↵")
                            .font(.system(size: 10, weight: .bold, design: .monospaced))
                            .padding(.horizontal, 4)
                            .padding(.vertical, 1)
                            .background(RoundedRectangle(cornerRadius: 3).fill(Color.white.opacity(0.1)))
                        Text("Select")
                            .font(.system(size: 10.5))
                            .foregroundColor(.secondary)
                    }
                    Spacer()
                    Text("Loop LAC Studio Spotlight")
                        .font(.system(size: 10))
                        .foregroundColor(.secondary.opacity(0.6))
                }
                .padding(.horizontal, 14)
                .padding(.vertical, 8)
                .background(Color.black.opacity(0.25))
            }
            .frame(width: 560)
            .background(
                RoundedRectangle(cornerRadius: 18, style: .continuous)
                    .fill(.ultraThinMaterial)
                    .overlay(LiquidGlass.specularSheen(cornerRadius: 18))
                    .overlay(LiquidGlass.specularBorder(cornerRadius: 18, isHovered: false))
            )
            .clipShape(RoundedRectangle(cornerRadius: 18, style: .continuous))
            .shadow(color: Color.black.opacity(0.35), radius: 32, x: 0, y: 16)
            .shadow(color: Color.black.opacity(0.15), radius: 8, x: 0, y: 3)
            .padding(.top, 60)
            .frame(maxHeight: .infinity, alignment: .top)
        }
        .onChange(of: query) { _ in
            selectedIndex = 0
        }
        .background {
            Group {
                Button("") {
                    dismiss()
                }
                .keyboardShortcut(.escape, modifiers: [])

                Button("") {
                    moveSelection(by: -1)
                }
                .keyboardShortcut(.upArrow, modifiers: [])

                Button("") {
                    moveSelection(by: 1)
                }
                .keyboardShortcut(.downArrow, modifiers: [])
            }
            .opacity(0)
            .allowsHitTesting(false)
        }
    }

    private func itemRow(item: PaletteItem, isSelected: Bool) -> some View {
        HStack(spacing: 12) {
            Image(systemName: item.icon)
                .font(.system(size: 13, weight: .medium))
                .foregroundColor(isSelected ? .accentColor : .secondary)
                .frame(width: 22)

            VStack(alignment: .leading, spacing: 2) {
                Text(item.title)
                    .font(.system(size: 12.5, weight: isSelected ? .semibold : .regular))
                    .foregroundColor(.primary)
                if let sub = item.subtitle {
                    Text(sub)
                        .font(.system(size: 10.5))
                        .foregroundColor(.secondary)
                }
            }

            Spacer()

            if let shortcut = item.shortcut {
                Text(shortcut)
                    .font(.system(size: 11, weight: .medium, design: .monospaced))
                    .foregroundColor(.secondary.opacity(0.8))
                    .padding(.horizontal, 6)
                    .padding(.vertical, 2)
                    .background(RoundedRectangle(cornerRadius: 4).fill(Color.white.opacity(0.08)))
            } else {
                Text(item.category)
                    .font(.system(size: 9.5, weight: .medium))
                    .foregroundColor(.secondary.opacity(0.6))
                    .padding(.horizontal, 6)
                    .padding(.vertical, 2)
                    .background(Capsule().fill(Color.white.opacity(0.05)))
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .background(
            RoundedRectangle(cornerRadius: 10, style: .continuous)
                .fill(isSelected ? Color.accentColor.opacity(0.18) : Color.clear)
                .overlay(
                    RoundedRectangle(cornerRadius: 10, style: .continuous)
                        .strokeBorder(isSelected ? Color.accentColor.opacity(0.3) : Color.clear, lineWidth: 1)
                )
        )
        .contentShape(Rectangle())
    }

    private func executeSelected() {
        let items = filteredItems
        guard !items.isEmpty, selectedIndex < items.count else { return }
        execute(items[selectedIndex])
    }

    private func moveSelection(by delta: Int) {
        let count = filteredItems.count
        guard count > 0 else { return }
        selectedIndex = min(max(0, selectedIndex + delta), count - 1)
    }

    private func execute(_ item: PaletteItem) {
        LiquidGlass.haptic(.alignment)
        item.action()
        dismiss()
    }

    private func dismiss() {
        withAnimation(LiquidGlass.spring) {
            isPresented = false
        }
    }
}
