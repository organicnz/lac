import AppKit
import SwiftUI

// MARK: - Loops & Kanban Canvas (AGENTS.md Autonomous Workflows)

public struct LoopsView: View {
    @ObservedObject var store: LoopsStore
    @Binding var sidebarVisibility: NavigationSplitViewVisibility

    @State private var isNewTaskPresented: Bool = false
    @State private var isDiffSheetPresented: Bool = false
    @State private var activeGitDiff: String = ""
    @State private var isDiffLoading: Bool = false
    @State private var isLogDrawerExpanded: Bool = true
    @State private var newTaskDescription: String = ""
    @State private var newTaskPriority: TaskPriority = .high
    @State private var newTaskLoop: String = "daily-coding.yaml"
    @State private var newTaskModule: String = ""
    @FocusState private var isTaskDescriptionFocused: Bool
    @FocusState private var isModuleFocused: Bool

    public init(
        store: LoopsStore,
        sidebarVisibility: Binding<NavigationSplitViewVisibility> = .constant(.all)
    ) {
        self.store = store
        self._sidebarVisibility = sidebarVisibility
    }

    public var body: some View {
        VStack(spacing: 0) {
            headerBar
            Divider().opacity(0.3)

            // Loop Pipeline Timeline
            if let loop = store.selectedLoop {
                loopPipelineBar(loop)
                Divider().opacity(0.25)
            }

            // Main Kanban Board
            kanbanBoardView
                .frame(maxWidth: .infinity, maxHeight: .infinity)

            // Execution & Human Gate Drawer
            if store.isRunningLoop || store.isGateAwaitingApproval || !store.runnerLogs.isEmpty {
                Divider().opacity(0.3)
                runnerDrawer
            }
        }
        .background(VisualEffectView().ignoresSafeArea())
        .sheet(isPresented: $isNewTaskPresented) {
            newTaskSheet
        }
        .sheet(isPresented: $isDiffSheetPresented) {
            gitDiffSheet
        }
    }

    // MARK: - Header Bar

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
                    Text("Autonomous Loops")
                        .font(.system(size: 13, weight: .bold))
                    Text("KANBAN")
                        .font(.system(size: 9, weight: .bold, design: .monospaced))
                        .padding(.horizontal, 5)
                        .padding(.vertical, 1)
                        .background(Capsule().fill(Color.accentColor.opacity(0.2)))
                        .foregroundColor(.accentColor)
                }
                Text("Autonomous task queue & reliability gates (~/todo/lac-tasks.yaml)")
                    .font(.system(size: 10))
                    .foregroundColor(.secondary)
            }

            Spacer()

            // Loop Selector Pill
            Menu {
                ForEach(store.loops) { loop in
                    Button(loop.name) {
                        store.selectedLoop = loop
                        LiquidGlass.haptic(.alignment)
                    }
                }
            } label: {
                HStack(spacing: 5) {
                    Image(systemName: "arrow.triangle.2.circlepath")
                        .font(.system(size: 10, weight: .semibold))
                        .foregroundColor(.accentColor)
                    Text(store.selectedLoop?.name ?? "daily-coding.yaml")
                        .font(.system(size: 11, weight: .medium))
                    Image(systemName: "chevron.down")
                        .font(.system(size: 9))
                        .foregroundColor(.secondary)
                }
                .claudeModelPill()
            }
            .menuStyle(.borderlessButton)
            .fixedSize()

            // New Task Button
            Button {
                isNewTaskPresented = true
                LiquidGlass.haptic(.alignment)
            } label: {
                HStack(spacing: 4) {
                    Image(systemName: "plus")
                        .font(.system(size: 10, weight: .bold))
                    Text("New Task")
                        .font(.system(size: 11, weight: .medium))
                }
            }
            .controlSize(.small)
            .lacGlass()

            // Run Top Pending Task
            Button {
                if let next = store.tasks(for: .pending).first {
                    store.executeLoop(task: next)
                }
            } label: {
                HStack(spacing: 5) {
                    Image(systemName: "play.fill")
                        .font(.system(size: 9))
                    Text("Execute Loop")
                        .font(.system(size: 11, weight: .semibold))
                }
            }
            .controlSize(.small)
            .lacGlassProminent()
            .disabled(store.isRunningLoop || store.tasks(for: .pending).isEmpty)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
        .background(.ultraThinMaterial)
    }

    // MARK: - Loop Pipeline Timeline

    private func loopPipelineBar(_ loop: LoopDefinition) -> some View {
        HStack(spacing: 12) {
            Text(loop.description)
                .font(.system(size: 11))
                .foregroundColor(.secondary)
                .lineLimit(1)

            Spacer()

            HStack(spacing: 4) {
                ForEach(Array(loop.phases.enumerated()), id: \.offset) { idx, phase in
                    HStack(spacing: 3) {
                        if phase.isHumanGate {
                            Image(systemName: "hand.raised.fill")
                                .font(.system(size: 8))
                                .foregroundColor(.yellow)
                        }
                        Text(phase.name)
                            .font(.system(size: 9, weight: .semibold, design: .monospaced))
                            .foregroundColor(phaseColor(phase.name))
                    }
                    .padding(.horizontal, 6)
                    .padding(.vertical, 2)
                    .background(
                        Capsule()
                            .fill(phaseBackground(phase.name))
                            .overlay(Capsule().strokeBorder(phaseBorder(phase.name), lineWidth: 0.5))
                    )

                    if idx < loop.phases.count - 1 {
                        Image(systemName: "chevron.right")
                            .font(.system(size: 6, weight: .bold))
                            .foregroundColor(.secondary.opacity(0.35))
                    }
                }
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 6)
        .background(Color.black.opacity(0.18))
    }

    private func phaseColor(_ name: String) -> Color {
        let active = store.currentRunnerPhase.rawValue.lowercased()
        if active.contains(name.lowercased()) {
            return .accentColor
        }
        return .primary.opacity(0.8)
    }

    private func phaseBackground(_ name: String) -> Color {
        let active = store.currentRunnerPhase.rawValue.lowercased()
        if active.contains(name.lowercased()) {
            return Color.accentColor.opacity(0.20)
        }
        return Color.white.opacity(0.04)
    }

    private func phaseBorder(_ name: String) -> Color {
        let active = store.currentRunnerPhase.rawValue.lowercased()
        if active.contains(name.lowercased()) {
            return Color.accentColor.opacity(0.50)
        }
        return Color.white.opacity(0.08)
    }

    // MARK: - Kanban Board

    private var kanbanBoardView: some View {
        HStack(alignment: .top, spacing: 12) {
            kanbanColumn(status: .pending, title: "Task Queue", color: .secondary)
            kanbanColumn(status: .inProgress, title: "In Progress", color: .blue)
            kanbanColumn(status: .reviewGate, title: "Review & Gate", color: .orange)
            kanbanColumn(status: .completed, title: "Completed", color: .green)
        }
        .padding(14)
    }

    private func kanbanColumn(status: TaskStatus, title: String, color: Color) -> some View {
        let list = store.tasks(for: status)

        return VStack(alignment: .leading, spacing: 10) {
            // Column Header
            HStack(spacing: 6) {
                Image(systemName: status.icon)
                    .font(.system(size: 11))
                    .foregroundColor(color)

                Text(title)
                    .font(.system(size: 12, weight: .bold))
                    .foregroundColor(.primary)

                Spacer()

                Text("\(list.count)")
                    .font(.system(size: 10, weight: .bold, design: .monospaced))
                    .foregroundColor(color)
                    .padding(.horizontal, 6)
                    .padding(.vertical, 1)
                    .background(Capsule().fill(color.opacity(0.15)))
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 6)
            .background(
                RoundedRectangle(cornerRadius: 8, style: .continuous)
                    .fill(Color.white.opacity(0.04))
            )

            // Cards Scroll
            ScrollView(.vertical, showsIndicators: false) {
                LazyVStack(spacing: 8) {
                    if list.isEmpty {
                        VStack(spacing: 4) {
                            Spacer().frame(height: 30)
                            Text("No tasks")
                                .font(.system(size: 11))
                                .foregroundColor(.secondary.opacity(0.5))
                            Spacer().frame(height: 30)
                        }
                        .frame(maxWidth: .infinity)
                    } else {
                        ForEach(list) { task in
                            taskCard(task)
                        }
                    }
                }
            }
        }
        .padding(8)
        .frame(minWidth: 200, maxWidth: .infinity)
        .background(
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .fill(Color.black.opacity(0.18))
                .overlay(RoundedRectangle(cornerRadius: 12).strokeBorder(Color.white.opacity(0.06), lineWidth: 1))
        )
    }

    private func taskCard(_ task: TaskItem) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            // Card Header: ID & Priority
            HStack(spacing: 6) {
                Text(task.id)
                    .font(.system(size: 10.5, weight: .bold, design: .monospaced))
                    .foregroundColor(.secondary)

                if let mod = task.module {
                    Text(mod)
                        .font(.system(size: 9, design: .monospaced))
                        .foregroundColor(.cyan)
                        .padding(.horizontal, 4)
                        .padding(.vertical, 1)
                        .background(Capsule().fill(Color.cyan.opacity(0.12)))
                }

                Spacer()

                Text(task.priority.label)
                    .font(.system(size: 9, weight: .bold))
                    .foregroundColor(task.priority.color)
                    .padding(.horizontal, 5)
                    .padding(.vertical, 1.5)
                    .background(Capsule().fill(task.priority.color.opacity(0.15)))
            }

            // Task Content
            Text(task.task)
                .font(.system(size: 12, weight: .medium))
                .foregroundColor(.primary.opacity(0.95))
                .lineLimit(3)
                .fixedSize(horizontal: false, vertical: true)

            // Footer: Loop Pill & Action Buttons
            HStack {
                HStack(spacing: 3) {
                    Image(systemName: "arrow.triangle.2.circlepath")
                        .font(.system(size: 8))
                    Text(task.loopName.replacingOccurrences(of: ".yaml", with: ""))
                        .font(.system(size: 9, design: .monospaced))
                }
                .foregroundColor(.secondary)

                Spacer()

                // Move Task Menu
                Menu {
                    ForEach(TaskStatus.allCases) { s in
                        if s != task.status {
                            Button("Move to \(s.title)") {
                                store.moveTask(id: task.id, to: s)
                            }
                        }
                    }
                    Divider()
                    Button("Delete Task", role: .destructive) {
                        store.deleteTask(id: task.id)
                    }
                } label: {
                    Image(systemName: "ellipsis")
                        .font(.system(size: 9, weight: .bold))
                        .foregroundColor(.secondary)
                        .frame(width: 20, height: 20)
                        .background(Circle().fill(Color.white.opacity(0.06)))
                }
                .menuStyle(.borderlessButton)
                .fixedSize()

                // Execute Loop for this card
                if task.status != .completed {
                    Button {
                        store.executeLoop(task: task)
                    } label: {
                        Image(systemName: "play.circle.fill")
                            .font(.system(size: 13))
                            .foregroundColor(.accentColor)
                    }
                    .buttonStyle(.plain)
                    .help("Execute autonomous loop on this task")
                }
            }
        }
        .padding(10)
        .background(
            RoundedRectangle(cornerRadius: 8, style: .continuous)
                .fill(Color.black.opacity(0.35))
                .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(Color.white.opacity(0.10), lineWidth: 0.8))
        )
    }

    // MARK: - Runner & Human Gate Drawer

    private var runnerDrawer: some View {
        VStack(spacing: 0) {
            // Drawer Header Bar
            HStack(spacing: 8) {
                Image(systemName: store.currentRunnerPhase.icon)
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundColor(store.isGateAwaitingApproval ? .yellow : .accentColor)

                Text("Loop Runner: \(store.currentRunnerPhase.rawValue)")
                    .font(.system(size: 12, weight: .bold))

                if store.isRunningLoop {
                    ProgressView().scaleEffect(0.6)
                }

                Spacer()

                if store.isGateAwaitingApproval {
                    HStack(spacing: 8) {
                        Text("Human Gate Gatekeeper: git diff review required")
                            .font(.system(size: 11, weight: .semibold))
                            .foregroundColor(.yellow)

                        Button {
                            loadGitDiff()
                        } label: {
                            HStack(spacing: 4) {
                                if isDiffLoading {
                                    ProgressView().controlSize(.mini)
                                } else {
                                    Image(systemName: "doc.text.magnifyingglass")
                                        .font(.system(size: 10))
                                }
                                Text("Inspect Git Diff")
                                    .font(.system(size: 11, weight: .semibold))
                            }
                        }
                        .controlSize(.small)
                        .lacGlass()
                        .disabled(isDiffLoading)

                        Button {
                            store.cancelLoop()
                        } label: {
                            Text("Reject")
                                .font(.system(size: 11))
                                .foregroundColor(.red)
                        }
                        .controlSize(.small)
                        .lacGlass()

                        Button {
                            store.approveHumanGate()
                        } label: {
                            HStack(spacing: 4) {
                                Image(systemName: "checkmark.seal.fill")
                                    .font(.system(size: 10))
                                Text("Approve & Complete")
                                    .font(.system(size: 11, weight: .bold))
                            }
                        }
                        .controlSize(.small)
                        .lacGlassProminent()
                    }
                } else {
                    Button {
                        withAnimation(LiquidGlass.spring) {
                            isLogDrawerExpanded.toggle()
                        }
                    } label: {
                        Image(systemName: isLogDrawerExpanded ? "chevron.down" : "chevron.up")
                            .font(.system(size: 10, weight: .semibold))
                            .foregroundColor(.secondary)
                    }
                    .buttonStyle(.plain)
                }
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 8)
            .background(Color.black.opacity(0.30))

            // Log output area
            if isLogDrawerExpanded {
                ScrollView(.vertical) {
                    Text(store.runnerLogs)
                        .font(.system(size: 11.5, design: .monospaced))
                        .foregroundColor(.primary.opacity(0.85))
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(12)
                        .textSelection(.enabled)
                }
                .frame(height: 160)
                .background(Color.black.opacity(0.50))
            }
        }
    }

    // MARK: - New Task Sheet

    private var newTaskSheet: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack {
                Text("Schedule Autonomous Task")
                    .font(.system(size: 14, weight: .bold))
                Spacer()
                Button("Cancel") {
                    isNewTaskPresented = false
                }
                .buttonStyle(.plain)
                .foregroundColor(.secondary)
            }

            Divider().opacity(0.3)

            VStack(alignment: .leading, spacing: 5) {
                Text("Task Description (Scope Tightly per AGENTS.md)")
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundColor(.secondary)
                TextEditor(text: $newTaskDescription)
                    .font(.system(size: 12, design: .monospaced))
                    .frame(height: 72)
                    .padding(8)
                    .background(
                        RoundedRectangle(cornerRadius: 8, style: .continuous)
                            .fill(Color.black.opacity(0.20))
                            .overlay(
                                RoundedRectangle(cornerRadius: 8, style: .continuous)
                                    .strokeBorder(
                                        isTaskDescriptionFocused
                                            ? LinearGradient(colors: [Color.accentColor, Color.purple], startPoint: .topLeading, endPoint: .bottomTrailing)
                                            : LinearGradient(colors: [Color.white.opacity(0.18), Color.white.opacity(0.06)], startPoint: .topLeading, endPoint: .bottomTrailing),
                                        lineWidth: isTaskDescriptionFocused ? 1.5 : 1
                                    )
                            )
                    )
                    .focused($isTaskDescriptionFocused)
            }

            HStack(spacing: 16) {
                VStack(alignment: .leading, spacing: 4) {
                    Text("Priority")
                        .font(.system(size: 11, weight: .semibold))
                        .foregroundColor(.secondary)
                    Picker("", selection: $newTaskPriority) {
                        ForEach(TaskPriority.allCases) { p in
                            Text(p.label).tag(p)
                        }
                    }
                    .pickerStyle(.segmented)
                    .frame(width: 220)
                }

                VStack(alignment: .leading, spacing: 4) {
                    Text("Target Loop")
                        .font(.system(size: 11, weight: .semibold))
                        .foregroundColor(.secondary)
                    Picker("", selection: $newTaskLoop) {
                        ForEach(store.loops) { l in
                            Text(l.name).tag(l.name)
                        }
                    }
                    .frame(width: 170)
                }
            }

            VStack(alignment: .leading, spacing: 5) {
                Text("Target Module / Scope (Optional)")
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundColor(.secondary)
                TextField("e.g. rust-src/gateway or SwiftUI/Chat", text: $newTaskModule)
                    .textFieldStyle(.plain)
                    .font(.system(size: 12))
                    .focused($isModuleFocused)
                    .liquidGlassTextField(cornerRadius: 8, isFocused: isModuleFocused)
            }

            HStack {
                Spacer()
                Button("Cancel") {
                    isNewTaskPresented = false
                }
                .keyboardShortcut(.cancelAction)
                .lacGlass()

                Button("Schedule Task") {
                    store.addTask(
                        taskDescription: newTaskDescription,
                        priority: newTaskPriority,
                        loopName: newTaskLoop,
                        module: newTaskModule
                    )
                    newTaskDescription = ""
                    newTaskModule = ""
                    isNewTaskPresented = false
                }
                .keyboardShortcut(.defaultAction)
                .controlSize(.regular)
                .lacGlassProminent()
                .disabled(newTaskDescription.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
        }
        .padding(22)
        .frame(width: 480)
        .liquidGlassModalCard(cornerRadius: 16)
        .onAppear {
            isTaskDescriptionFocused = true
        }
    }

    // MARK: - Git Diff Audit Sheet

    private var gitDiffSheet: some View {
        VStack(spacing: 0) {
            // Header
            HStack(spacing: 10) {
                Image(systemName: "arrow.triangle.branch")
                    .font(.system(size: 13, weight: .bold))
                    .foregroundColor(.accentColor)

                VStack(alignment: .leading, spacing: 1) {
                    Text("Phase 5 Human Gate: Working Tree Diff Review")
                        .font(.system(size: 13, weight: .bold))
                    Text("Verify changes uncommitted in working tree before approving completion (AGENTS.md mandatory gate)")
                        .font(.system(size: 10))
                        .foregroundColor(.secondary)
                }

                Spacer()

                Button {
                    loadGitDiff()
                } label: {
                    Image(systemName: "arrow.clockwise")
                        .font(.system(size: 11))
                        .foregroundColor(.secondary)
                }
                .buttonStyle(.plain)
                .help("Refresh Diff")

                Button {
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString(activeGitDiff, forType: .string)
                    LiquidGlass.haptic(.alignment)
                } label: {
                    HStack(spacing: 4) {
                        Image(systemName: "doc.on.doc")
                            .font(.system(size: 10))
                        Text("Copy Diff")
                            .font(.system(size: 11))
                    }
                }
                .controlSize(.small)
                .lacGlass()

                Button {
                    store.approveHumanGate()
                    isDiffSheetPresented = false
                } label: {
                    HStack(spacing: 4) {
                        Image(systemName: "checkmark.seal.fill")
                            .font(.system(size: 10))
                        Text("Approve Gate")
                            .font(.system(size: 11, weight: .bold))
                    }
                }
                .keyboardShortcut(.defaultAction)
                .controlSize(.small)
                .lacGlassProminent()

                Button {
                    isDiffSheetPresented = false
                } label: {
                    Image(systemName: "xmark.circle.fill")
                        .font(.system(size: 14))
                        .foregroundColor(.secondary)
                }
                .keyboardShortcut(.cancelAction)
                .buttonStyle(.plain)
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 12)
            .background(.ultraThinMaterial)
            .overlay(Divider().opacity(0.3), alignment: .bottom)

            // Diff Body
            ScrollView(.vertical, showsIndicators: true) {
                if activeGitDiff.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    VStack(spacing: 8) {
                        Spacer().frame(height: 60)
                        Image(systemName: "checkmark.circle.fill")
                            .font(.system(size: 28))
                            .foregroundColor(.green)
                        Text("Working tree is completely clean.")
                            .font(.system(size: 13, weight: .semibold))
                        Text("No unstaged or uncommitted changes detected in repository.")
                            .font(.system(size: 11))
                            .foregroundColor(.secondary)
                        Spacer().frame(height: 60)
                    }
                    .frame(maxWidth: .infinity)
                } else {
                    LazyVStack(alignment: .leading, spacing: 1) {
                        ForEach(Array(activeGitDiff.components(separatedBy: "\n").enumerated()), id: \.offset) { _, line in
                            let isAdd = line.hasPrefix("+") && !line.hasPrefix("+++")
                            let isDel = line.hasPrefix("-") && !line.hasPrefix("---")
                            let isHunk = line.hasPrefix("@@")
                            let isHeader = line.hasPrefix("diff --git") || line.hasPrefix("index ")

                            HStack(spacing: 8) {
                                Text(line)
                                    .font(.system(size: 11, design: .monospaced))
                                    .foregroundColor(
                                        isAdd ? .green :
                                        isDel ? .red :
                                        isHunk ? .cyan :
                                        isHeader ? .yellow : .primary.opacity(0.9)
                                    )
                                    .frame(maxWidth: .infinity, alignment: .leading)
                            }
                            .padding(.horizontal, 8)
                            .padding(.vertical, 1)
                            .background(
                                isAdd ? Color.green.opacity(0.12) :
                                isDel ? Color.red.opacity(0.12) :
                                isHunk ? Color.cyan.opacity(0.08) :
                                isHeader ? Color.yellow.opacity(0.08) : Color.clear
                            )
                        }
                    }
                    .padding(10)
                }
            }
            .background(Color.black.opacity(0.40))
        }
        .frame(width: 740, height: 520)
        .liquidGlassModalCard(cornerRadius: 16)
    }

    private func loadGitDiff() {
        isDiffLoading = true
        let cwd = FileManager.default.currentDirectoryPath
        let root = URL(fileURLWithPath: cwd)

        Task.detached(priority: .userInitiated) {
            let proc = Process()
            proc.executableURL = URL(fileURLWithPath: "/usr/bin/git")
            proc.arguments = ["diff"]
            proc.currentDirectoryURL = root

            let pipe = Pipe()
            proc.standardOutput = pipe
            proc.standardError = Pipe()

            do {
                try proc.run()
                proc.waitUntilExit()
                let data = pipe.fileHandleForReading.readDataToEndOfFile()
                let diffText = String(data: data, encoding: .utf8) ?? ""
                await MainActor.run {
                    self.activeGitDiff = diffText
                    self.isDiffLoading = false
                    self.isDiffSheetPresented = true
                    LiquidGlass.haptic(.alignment)
                }
            } catch {
                await MainActor.run {
                    self.activeGitDiff = "Failed to load git diff: \(error.localizedDescription)"
                    self.isDiffLoading = false
                    self.isDiffSheetPresented = true
                }
            }
        }
    }
}
