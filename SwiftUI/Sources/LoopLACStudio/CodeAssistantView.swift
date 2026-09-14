import AppKit
import SwiftUI

// MARK: - Code Assistant View (Codex / Claude Desktop style Coding Canvas)

public struct CodeAssistantView: View {
    @ObservedObject var store: CodeAssistantStore
    @Binding var sidebarVisibility: NavigationSplitViewVisibility
    @StateObject private var consoleStore = AgentConsoleStore()
    @State private var copiedResponse = false
    @State private var appliedNotice = false
    @State private var viewMode: AssistantViewMode = .code
    @State private var isFileTreeVisible: Bool = false
    @State private var isConsoleVisible: Bool = false
    @State private var isQuickOpenPresented: Bool = false
    @AppStorage("codeEditorFontSize") private var editorFontSize: Double = 12.5
    @State private var workspaceFolder: URL = {
        let cwd = FileManager.default.currentDirectoryPath
        return URL(fileURLWithPath: cwd)
    }()

    enum AssistantViewMode: String, CaseIterable, Identifiable {
        case code = "Code"
        case diff = "Diff"
        var id: String { rawValue }
    }

    public init(
        store: CodeAssistantStore,
        sidebarVisibility: Binding<NavigationSplitViewVisibility> = .constant(.all)
    ) {
        self.store = store
        self._sidebarVisibility = sidebarVisibility
    }

    public var body: some View {
        ZStack {
            VStack(spacing: 0) {
                headerBar
                Divider().opacity(0.3)

                // Main Split Workbench
                HSplitView {
                    if isFileTreeVisible {
                        WorkspaceFileTree(rootDirectory: $workspaceFolder) { fileUrl, content in
                            store.openFile(url: fileUrl, content: content)
                        }
                    }

                    leftEditorCanvas
                        .frame(minWidth: 320, idealWidth: 420)

                    rightAssistantCanvas
                        .frame(minWidth: 380, idealWidth: 500)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)

                if isConsoleVisible {
                    AgentConsoleView(
                        console: consoleStore,
                        workspaceRoot: workspaceFolder,
                        onClose: {
                            withAnimation(LiquidGlass.spring) {
                                isConsoleVisible = false
                            }
                        }
                    )
                }

                Divider().opacity(0.3)
                bottomPromptBar
            }
            .background(VisualEffectView().ignoresSafeArea())

            // Quick Open File Modal Overlay (⌘P)
            if isQuickOpenPresented {
                QuickFileOpenView(
                    isPresented: $isQuickOpenPresented,
                    rootUrl: workspaceFolder
                ) { fileUrl, content in
                    store.openFile(url: fileUrl, content: content)
                }
                .transition(.opacity.combined(with: .scale(scale: 0.98)))
                .zIndex(100)
            }
        }
        .animation(LiquidGlass.spring, value: isQuickOpenPresented)
        .onChange(of: store.pendingConsoleCommand) { cmd in
            if let cmd { consumeConsoleCommand(cmd) }
        }
        .onAppear {
            // Palette fired while this canvas was off-screen: run on arrival.
            if let cmd = store.pendingConsoleCommand { consumeConsoleCommand(cmd) }
        }
    }

    // MARK: Palette → Console Bridge (⌘K actions run for real)

    private func consumeConsoleCommand(_ cmd: ConsoleCommand) {
        store.pendingConsoleCommand = nil
        withAnimation(LiquidGlass.spring) { isConsoleVisible = true }
        LiquidGlass.haptic(.alignment)
        switch cmd {
        case .cargoTests: consoleStore.runCargoTests(repoRoot: workspaceFolder)
        case .swiftTests: consoleStore.runSwiftTests(repoRoot: workspaceFolder)
        case .gitDiff: consoleStore.runGitDiff(repoRoot: workspaceFolder)
        }
    }

    // MARK: Header Bar

    private var headerBar: some View {
        HStack(spacing: 12) {
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
                HStack(spacing: 6) {
                    Text("Code Assistant")
                        .font(.system(size: 13, weight: .bold))
                    Text("AGENTIC")
                        .font(.system(size: 9, weight: .bold, design: .monospaced))
                        .padding(.horizontal, 5)
                        .padding(.vertical, 1)
                        .background(Capsule().fill(Color.accentColor.opacity(0.2)))
                        .foregroundColor(.accentColor)

                    // Agentic Pipeline Steps
                    HStack(spacing: 3) {
                        ForEach(["Plan", "Inspect", "Diff", "Apply", "Verify"], id: \.self) { step in
                            Text(step)
                                .font(.system(size: 8, weight: .semibold, design: .monospaced))
                                .foregroundColor(workflowStepColor(step))
                                .padding(.horizontal, 4)
                                .padding(.vertical, 1)
                                .background(Capsule().fill(workflowStepBackground(step)))
                            if step != "Verify" {
                                Image(systemName: "chevron.right")
                                    .font(.system(size: 5, weight: .bold))
                                    .foregroundColor(.secondary.opacity(0.35))
                            }
                        }
                    }
                    .padding(.horizontal, 5)
                    .padding(.vertical, 2)
                    .background(Capsule().fill(Color.white.opacity(0.04)))
                }
                Text("Autonomous systems engineering & refactoring on Apple Silicon")
                    .font(.system(size: 10))
                    .foregroundColor(.secondary)
            }

            Spacer()

            // Quick Open (⌘P)
            Button {
                withAnimation(LiquidGlass.spring) {
                    isQuickOpenPresented.toggle()
                }
                LiquidGlass.haptic(.alignment)
            } label: {
                HStack(spacing: 4) {
                    Image(systemName: "magnifyingglass")
                        .font(.system(size: 10))
                    Text("Quick Open")
                        .font(.system(size: 11))
                }
            }
            .controlSize(.small)
            .lacGlass()
            .help("Quickly find and open any workspace file (⌘P)")
            .keyboardShortcut("p", modifiers: .command)

            // File Tree Toggle
            Button {
                withAnimation(LiquidGlass.spring) {
                    isFileTreeVisible.toggle()
                }
                LiquidGlass.haptic(.alignment)
            } label: {
                HStack(spacing: 4) {
                    Image(systemName: isFileTreeVisible ? "sidebar.left" : "folder")
                        .font(.system(size: 10))
                    Text(isFileTreeVisible ? "Hide Files" : "Files")
                        .font(.system(size: 11))
                }
            }
            .controlSize(.small)
            .lacGlass()
            .help("Toggle Workspace File Browser (⌥⌘F)")
            .keyboardShortcut("f", modifiers: [.command, .option])

            // Agent Console & Test Runner Toggle
            Button {
                withAnimation(LiquidGlass.spring) {
                    isConsoleVisible.toggle()
                }
                LiquidGlass.haptic(.alignment)
            } label: {
                HStack(spacing: 4) {
                    Image(systemName: isConsoleVisible ? "terminal.fill" : "terminal")
                        .font(.system(size: 10))
                    Text(isConsoleVisible ? "Hide Console" : "Console")
                        .font(.system(size: 11))
                }
            }
            .controlSize(.small)
            .lacGlass()
            .help("Toggle Agent Console & Test Runner (⌥⌘T)")
            .keyboardShortcut("t", modifiers: [.command, .option])

            // Language picker
            Menu {
                ForEach(CodeAssistantStore.availableLanguages, id: \.self) { lang in
                    Button(lang) {
                        store.selectedLanguage = lang
                        LiquidGlass.haptic(.alignment)
                    }
                }
            } label: {
                HStack(spacing: 5) {
                    Image(systemName: "chevron.left.forwardslash.chevron.right")
                        .font(.system(size: 10, weight: .semibold))
                    Text(store.selectedLanguage)
                        .font(.system(size: 11, weight: .medium))
                    Image(systemName: "chevron.down")
                        .font(.system(size: 9))
                        .foregroundColor(.secondary)
                }
                .claudeModelPill()
            }
            .menuStyle(.borderlessButton)
            .fixedSize()

            // Open file from disk
            Button {
                let panel = NSOpenPanel()
                panel.allowsMultipleSelection = false
                panel.canChooseDirectories = false
                panel.canChooseFiles = true
                if panel.runModal() == .OK, let url = panel.url {
                    if let content = try? String(contentsOf: url, encoding: .utf8) {
                        store.openFile(url: url, content: content)
                        LiquidGlass.haptic(.alignment)
                    }
                }
            } label: {
                HStack(spacing: 4) {
                    Image(systemName: "folder")
                        .font(.system(size: 10))
                    Text("Open File...")
                        .font(.system(size: 11))
                }
            }
            .controlSize(.small)
            .lacGlass()
            .help("Load a workspace file directly into the editor")

            // Quick Save active file button
            if let active = store.activeTab, active.url != nil {
                Button {
                    store.saveActiveFile()
                } label: {
                    HStack(spacing: 4) {
                        Image(systemName: "square.and.arrow.down")
                            .font(.system(size: 10))
                        Text("Save")
                            .font(.system(size: 11))
                        if active.isModified {
                            Circle()
                                .fill(Color.orange)
                                .frame(width: 5, height: 5)
                        }
                    }
                }
                .controlSize(.small)
                .lacGlass()
                .help("Save active file to disk (⌘S)")
                .keyboardShortcut("s", modifiers: .command)
            }

            // Undo Apply button if available
            if store.lastAppliedCode != nil {
                Button {
                    store.undoApply()
                } label: {
                    HStack(spacing: 4) {
                        Image(systemName: "arrow.uturn.backward")
                            .font(.system(size: 10))
                        Text("Undo Apply")
                            .font(.system(size: 11))
                    }
                }
                .controlSize(.small)
                .lacGlass()
                .help("Revert editor to code before last assistant apply")
            }

            // Clear editor
            Button {
                LiquidGlass.haptic(.alignment)
                store.sourceCode = ""
            } label: {
                Image(systemName: "trash")
                    .font(.system(size: 11))
            }
            .controlSize(.small)
            .lacGlass()
            .help("Clear editor")
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
        .background(.ultraThinMaterial)
    }

    // MARK: Left Editor Canvas

    private func iconForTab(_ tab: EditorTab) -> String {
        let ext = (tab.url?.pathExtension ?? "").lowercased()
        switch ext {
        case "rs": return "cpu"
        case "swift": return "swift"
        case "py": return "terminal"
        case "ts", "tsx", "js": return "curlybraces"
        case "json", "toml", "yaml": return "doc.badge.gearshape"
        case "sh": return "terminal.fill"
        default: return "doc.text.fill"
        }
    }

    private var leftEditorCanvas: some View {
        VStack(spacing: 0) {
            // Working Set Tabs Bar
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 4) {
                    ForEach(store.tabs) { tab in
                        HStack(spacing: 6) {
                            Image(systemName: iconForTab(tab))
                                .font(.system(size: 10))
                                .foregroundColor(tab.id == store.activeTabId ? .accentColor : .secondary)

                            Text(tab.title)
                                .font(.system(size: 11.5, weight: tab.id == store.activeTabId ? .semibold : .regular))
                                .foregroundColor(tab.id == store.activeTabId ? .primary : .secondary)

                            if tab.isModified {
                                Circle()
                                    .fill(Color.orange)
                                    .frame(width: 5, height: 5)
                            }

                            Button {
                                LiquidGlass.haptic(.alignment)
                                store.closeTab(id: tab.id)
                            } label: {
                                Image(systemName: "xmark")
                                    .font(.system(size: 8, weight: .bold))
                                    .foregroundColor(.secondary.opacity(0.7))
                                    .padding(3)
                                    .background(Circle().fill(Color.white.opacity(0.06)))
                            }
                            .buttonStyle(.plain)
                        }
                        .padding(.horizontal, 9)
                        .padding(.vertical, 5)
                        .background(
                            RoundedRectangle(cornerRadius: 6, style: .continuous)
                                .fill(tab.id == store.activeTabId ? Color.white.opacity(0.12) : Color.white.opacity(0.03))
                                .overlay(
                                    RoundedRectangle(cornerRadius: 6, style: .continuous)
                                        .strokeBorder(tab.id == store.activeTabId ? Color.white.opacity(0.18) : Color.clear, lineWidth: 1)
                                )
                        )
                        .contentShape(Rectangle())
                        .onTapGesture {
                            LiquidGlass.haptic(.alignment)
                            store.selectTab(id: tab.id)
                        }
                        .contextMenu {
                            Button("Close Tab") {
                                store.closeTab(id: tab.id)
                            }
                            Button("Close Other Tabs") {
                                store.closeOtherTabs(id: tab.id)
                            }
                            Button("Close All Tabs") {
                                store.closeAllTabs()
                            }
                            Divider()
                            if let url = tab.url {
                                Button("Reveal in Finder") {
                                    NSWorkspace.shared.activateFileViewerSelecting([url])
                                }
                                Button("Copy Path") {
                                    NSPasteboard.general.clearContents()
                                    NSPasteboard.general.setString(url.path, forType: .string)
                                }
                            }
                        }
                    }

                    // New tab button
                    Button {
                        LiquidGlass.haptic(.alignment)
                        store.newTab()
                    } label: {
                        Image(systemName: "plus")
                            .font(.system(size: 10, weight: .medium))
                            .foregroundColor(.secondary)
                            .frame(width: 22, height: 22)
                            .background(
                                RoundedRectangle(cornerRadius: 5, style: .continuous)
                                    .fill(Color.white.opacity(0.04))
                            )
                    }
                    .buttonStyle(.plain)
                    .help("New Untitled Tab")
                }
                .padding(.horizontal, 10)
                .padding(.vertical, 5)
            }
            .background(Color.black.opacity(0.25))
            .overlay(Divider().opacity(0.25), alignment: .bottom)

            // Action capsules bar
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 8) {
                    ForEach(CodeAssistantAction.allCases) { action in
                        Button {
                            LiquidGlass.haptic(.alignment)
                            store.executeAction(action)
                        } label: {
                            HStack(spacing: 5) {
                                Image(systemName: action.icon)
                                    .font(.system(size: 10))
                                Text(action.rawValue)
                                    .font(.system(size: 11, weight: .medium))
                            }
                            .padding(.horizontal, 10)
                            .padding(.vertical, 5)
                            .background(
                                Capsule()
                                    .fill(.ultraThinMaterial)
                                    .overlay(
                                        Capsule()
                                            .strokeBorder(Color.white.opacity(0.18), lineWidth: 1)
                                    )
                            )
                        }
                        .buttonStyle(.plain)
                        .disabled(store.isProcessing)
                    }
                }
                .padding(.horizontal, 14)
                .padding(.vertical, 8)
            }
            .background(Color.black.opacity(0.15))
            .overlay(Divider().opacity(0.2), alignment: .bottom)

            // Monospaced Code Editor with Line Number Gutter
            HStack(alignment: .top, spacing: 0) {
                let lines = store.sourceCode.components(separatedBy: "\n")
                let lineCount = max(1, lines.count)
                let lineHeight = CGFloat(max(15.0, editorFontSize * 1.44))

                // Line Number Gutter
                ScrollView(.vertical, showsIndicators: false) {
                    VStack(alignment: .trailing, spacing: 0) {
                        ForEach(1...lineCount, id: \.self) { lineNum in
                            Text("\(lineNum)")
                                .font(.system(size: CGFloat(editorFontSize), design: .monospaced))
                                .foregroundColor(.secondary.opacity(0.35))
                                .frame(height: lineHeight, alignment: .trailing)
                        }
                    }
                    .padding(.top, 14)
                    .padding(.leading, 6)
                    .padding(.trailing, 6)
                }
                .frame(width: max(32, CGFloat(String(lineCount).count * 8 + 16)))
                .background(Color.black.opacity(0.12))
                .overlay(Divider().opacity(0.18), alignment: .trailing)
                .allowsHitTesting(false)

                // Text Area
                ZStack(alignment: .topLeading) {
                    TextEditor(text: $store.sourceCode)
                        .font(.system(size: CGFloat(editorFontSize), design: .monospaced))
                        .padding(.horizontal, 12)
                        .padding(.vertical, 14)
                        .background(Color.clear)
                        .scrollContentBackground(.hidden)

                    if store.sourceCode.isEmpty {
                        Text("// Paste or type your \(store.selectedLanguage) code here…")
                            .font(.system(size: CGFloat(editorFontSize), design: .monospaced))
                            .foregroundColor(.secondary.opacity(0.5))
                            .padding(.horizontal, 16)
                            .padding(.vertical, 18)
                            .allowsHitTesting(false)
                    }
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)

            // Editor status footer
            HStack(spacing: 8) {
                Text("\(store.sourceCode.components(separatedBy: "\n").count) lines")
                    .font(.system(size: 10, design: .monospaced))
                    .foregroundColor(.secondary)
                Text("•")
                    .foregroundColor(.secondary.opacity(0.4))
                Text("\(store.sourceCode.count) chars")
                    .font(.system(size: 10, design: .monospaced))
                    .foregroundColor(.secondary)
                Text("•")
                    .foregroundColor(.secondary.opacity(0.4))
                Text("UTF-8")
                    .font(.system(size: 9.5, weight: .medium, design: .monospaced))
                    .foregroundColor(.secondary.opacity(0.8))
                Text("•")
                    .foregroundColor(.secondary.opacity(0.4))

                Text(store.selectedLanguage)
                    .font(.system(size: 9.5, weight: .semibold, design: .monospaced))
                    .foregroundColor(.accentColor)
                    .padding(.horizontal, 6)
                    .padding(.vertical, 1.5)
                    .background(Color.accentColor.opacity(0.12))
                    .clipShape(RoundedRectangle(cornerRadius: 3.5))

                // Font Size Stepper
                HStack(spacing: 3) {
                    Button(action: {
                        if editorFontSize > 9.5 { editorFontSize -= 1.0 }
                    }) {
                        Image(systemName: "minus")
                            .font(.system(size: 8, weight: .bold))
                            .frame(width: 14, height: 14)
                    }
                    .buttonStyle(.plain)
                    .help("Decrease Editor Font Size")

                    Text("\(Int(editorFontSize))pt")
                        .font(.system(size: 9.5, weight: .medium, design: .monospaced))
                        .foregroundColor(.secondary)

                    Button(action: {
                        if editorFontSize < 22.0 { editorFontSize += 1.0 }
                    }) {
                        Image(systemName: "plus")
                            .font(.system(size: 8, weight: .bold))
                            .frame(width: 14, height: 14)
                    }
                    .buttonStyle(.plain)
                    .help("Increase Editor Font Size")
                }
                .padding(.horizontal, 5)
                .padding(.vertical, 2)
                .background(Capsule().fill(Color.white.opacity(0.06)))

                Spacer()

                if store.tabs.contains(where: { $0.isModified }) {
                    Button(action: { store.saveAllTabs() }) {
                        HStack(spacing: 4) {
                            Image(systemName: "square.and.arrow.down.on.square.fill")
                                .font(.system(size: 9))
                            Text("Save All (⌥⌘S)")
                                .font(.system(size: 9.5, weight: .medium))
                        }
                        .foregroundColor(.accentColor)
                        .padding(.horizontal, 6)
                        .padding(.vertical, 2)
                        .background(Color.accentColor.opacity(0.14))
                        .clipShape(RoundedRectangle(cornerRadius: 4))
                    }
                    .buttonStyle(.plain)
                    .keyboardShortcut("s", modifiers: [.command, .option])
                }

                Button("Load RingBuffer Example") {
                    store.sourceCode = """
// LAC Native Worker: Zero-allocation circular buffer
pub struct RingBuffer<T, const N: usize> {
    data: [Option<T>; N],
    head: usize,
    tail: usize,
    len: usize,
}

impl<T: Copy, const N: usize> RingBuffer<T, N> {
    pub const fn new() -> Self {
        Self {
            data: [None; N],
            head: 0,
            tail: 0,
            len: 0,
        }
    }

    pub fn push(&mut self, item: T) -> Result<(), &'static str> {
        if self.len >= N {
            return Err("Buffer is full");
        }
        self.data[self.tail] = Some(item);
        self.tail = (self.tail + 1) % N;
        self.len += 1;
        Ok(())
    }

    pub fn pop(&mut self) -> Option<T> {
        if self.len == 0 {
            return None;
        }
        let item = self.data[self.head].take();
        self.head = (self.head + 1) % N;
        self.len -= 1;
        item
    }
}
"""
                }
                .font(.system(size: 10))
                .buttonStyle(.plain)
                .foregroundColor(.secondary.opacity(0.8))
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 6)
            .background(Color.black.opacity(0.20))
            .overlay(Divider().opacity(0.2), alignment: .top)
        }
        .background(Color.black.opacity(0.18))
    }

    // MARK: Right Assistant Canvas

    private var rightAssistantCanvas: some View {
        VStack(spacing: 0) {
            // Assistant header bar
            HStack {
                HStack(spacing: 6) {
                    Image(systemName: "sparkles")
                        .font(.system(size: 11, weight: .semibold))
                        .foregroundColor(.accentColor)
                    Text("Assistant Analysis")
                        .font(.system(size: 12, weight: .semibold))
                }

                Spacer()

                // Code vs Diff Mode Toggle when a code block is extracted
                if extractFirstCodeBlock(store.streamResponse) != nil {
                    Picker("View Mode", selection: $viewMode) {
                        ForEach(AssistantViewMode.allCases) { mode in
                            Text(mode.rawValue).tag(mode)
                        }
                    }
                    .pickerStyle(.segmented)
                    .frame(width: 130)
                }

                if store.isProcessing {
                    Button {
                        store.cancel()
                    } label: {
                        HStack(spacing: 4) {
                            Image(systemName: "stop.circle.fill")
                                .foregroundColor(.red)
                            Text("Stop")
                                .font(.system(size: 11, weight: .medium))
                                .foregroundColor(.red)
                        }
                    }
                    .controlSize(.small)
                    .lacGlass()
                }

                // Apply extracted code to editor
                if let extracted = extractFirstCodeBlock(store.streamResponse) {
                    Button {
                        store.applyToEditor(extracted)
                        withAnimation(LiquidGlass.spring) { appliedNotice = true }
                        DispatchQueue.main.asyncAfter(deadline: .now() + 1.8) {
                            withAnimation(LiquidGlass.spring) { appliedNotice = false }
                        }
                    } label: {
                        HStack(spacing: 4) {
                            Image(systemName: appliedNotice ? "checkmark" : "arrow.left.square.fill")
                                .font(.system(size: 10))
                                .foregroundColor(appliedNotice ? .green : .accentColor)
                            Text(appliedNotice ? "Applied!" : "Apply to Editor")
                                .font(.system(size: 11, weight: .semibold))
                                .foregroundColor(appliedNotice ? .green : .accentColor)
                        }
                    }
                    .controlSize(.small)
                    .lacGlass()
                    .help("Apply this code block directly to the left editor")
                }

                Button {
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString(store.streamResponse, forType: .string)
                    LiquidGlass.haptic(.alignment)
                    withAnimation(LiquidGlass.spring) { copiedResponse = true }
                    DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) {
                        withAnimation(LiquidGlass.spring) { copiedResponse = false }
                    }
                } label: {
                    HStack(spacing: 4) {
                        Image(systemName: copiedResponse ? "checkmark" : "doc.on.doc")
                            .font(.system(size: 10))
                        Text(copiedResponse ? "Copied" : "Copy")
                            .font(.system(size: 11))
                    }
                }
                .controlSize(.small)
                .lacGlass()
                .disabled(store.streamResponse.isEmpty)
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 8)
            .background(Color.black.opacity(0.15))
            .overlay(Divider().opacity(0.2), alignment: .bottom)

            // Assistant Output Body: Code View or Diff View
            if viewMode == .diff, let extracted = extractFirstCodeBlock(store.streamResponse) {
                CodeDiffView(
                    originalCode: store.sourceCode,
                    modifiedCode: extracted,
                    onAccept: {
                        store.applyToEditor(extracted)
                        withAnimation(LiquidGlass.spring) { viewMode = .code }
                    },
                    onApplyCode: { selectiveCode in
                        store.applyToEditor(selectiveCode)
                    }
                )
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                ScrollView {
                    VStack(alignment: .leading, spacing: 16) {
                        if store.streamResponse.isEmpty && !store.isProcessing {
                            emptyAssistantPlaceholder
                        } else {
                            MarkdownMessage(text: store.streamResponse)

                            if store.isProcessing {
                                HStack(spacing: 8) {
                                    ProgressView().scaleEffect(0.7)
                                    Text("Reasoning & generating code...")
                                        .font(.system(size: 12))
                                        .foregroundColor(.secondary)
                                }
                                .padding(.top, 8)
                            }
                        }

                        if let err = store.errorText {
                            VStack(alignment: .leading, spacing: 6) {
                                Text("Gateway Notice")
                                    .font(.system(size: 12, weight: .bold))
                                Text(err)
                                    .font(.system(size: 11))
                                    .foregroundColor(.secondary)
                            }
                            .padding(12)
                            .liquidGlassCard(cornerRadius: 10, hoverable: false, tint: .orange)
                        }
                    }
                    .padding(18)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .background(Color.black.opacity(0.12))
    }

    private var emptyAssistantPlaceholder: some View {
        VStack(spacing: 16) {
            Spacer(minLength: 40)
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
                            endRadius: 38
                        )
                    )
                    .frame(width: 68, height: 68)
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
                    .shadow(color: Color.accentColor.opacity(0.18), radius: 12, x: 0, y: 4)

                LACLogoView(size: 44, isAnimated: true)
            }
            Text("Loop Code Assistant")
                .font(.system(size: 17, weight: .bold, design: .rounded))
            Text("Select an action above (`Explain`, `Refactor`, `Generate Tests`, `Audit Bugs`, `Optimize`), load a file with `Open File...`, or type an instruction below.\n\nOptimized for Qwen 2.5 Coder 32B and Qwen 3.8 27B on Apple Silicon.")
                .font(.system(size: 12))
                .foregroundColor(.secondary)
                .multilineTextAlignment(.center)
                .lineSpacing(3)
                .frame(maxWidth: 400)
            Spacer()
        }
        .frame(maxWidth: .infinity)
    }

    // MARK: Bottom Custom Prompt Bar

    private var bottomPromptBar: some View {
        HStack(spacing: 10) {
            Image(systemName: "terminal")
                .foregroundColor(.secondary)
                .font(.system(size: 12))

            TextField("Ask Code Assistant to refactor, write a test, or debug this code (Return to run)…", text: $store.customPrompt)
                .textFieldStyle(.plain)
                .font(.system(size: 13))
                .onSubmit {
                    store.executeCustomPrompt()
                }

            if !store.customPrompt.isEmpty {
                Button {
                    store.customPrompt = ""
                } label: {
                    Image(systemName: "xmark.circle.fill")
                        .foregroundColor(.secondary)
                        .font(.system(size: 12))
                }
                .buttonStyle(.plain)
            }

            Button {
                store.executeCustomPrompt()
            } label: {
                Image(systemName: "arrow.up.circle.fill")
                    .font(.system(size: 22))
                    .foregroundColor(store.customPrompt.isEmpty ? .secondary.opacity(0.4) : .accentColor)
            }
            .buttonStyle(.plain)
            .disabled(store.customPrompt.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || store.isProcessing)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
        .background(.ultraThinMaterial)
    }

    // Helper to pull the primary code block from assistant output to allow 1-click apply
    private func extractFirstCodeBlock(_ text: String) -> String? {
        let lines = text.split(separator: "\n", omittingEmptySubsequences: false)
        var inside = false
        var blockLines: [String] = []
        for line in lines {
            let t = line.trimmingCharacters(in: .whitespaces)
            if t.hasPrefix("```") {
                if inside {
                    return blockLines.joined(separator: "\n")
                } else {
                    inside = true
                    continue
                }
            }
            if inside {
                blockLines.append(String(line))
            }
        }
        return nil
    }

    private func workflowStepColor(_ step: String) -> Color {
        if store.isProcessing {
            if step == "Diff" || step == "Apply" { return .accentColor }
            return .primary.opacity(0.8)
        }
        if viewMode == .diff && step == "Diff" { return .accentColor }
        if store.lastAppliedCode != nil && step == "Apply" { return .green }
        return .secondary.opacity(0.7)
    }

    private func workflowStepBackground(_ step: String) -> Color {
        if store.isProcessing && (step == "Diff" || step == "Apply") {
            return Color.accentColor.opacity(0.18)
        }
        if viewMode == .diff && step == "Diff" {
            return Color.accentColor.opacity(0.18)
        }
        if store.lastAppliedCode != nil && step == "Apply" {
            return Color.green.opacity(0.18)
        }
        return Color.white.opacity(0.04)
    }
}
