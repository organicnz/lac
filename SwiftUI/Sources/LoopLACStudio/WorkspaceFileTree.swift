import AppKit
import Foundation
import SwiftUI

// MARK: - Git File Status

public enum GitFileStatus: String, Sendable {
    case modified = "M"
    case untracked = "U"
    case added = "A"
    case deleted = "D"

    public var badgeLabel: String { rawValue }

    public var badgeColor: Color {
        switch self {
        case .modified: return .orange
        case .untracked, .added: return .green
        case .deleted: return .red
        }
    }
}

// MARK: - Workspace File Item

public struct WorkspaceFileItem: Identifiable, Hashable, Sendable {
    public var id: String { url.path }
    public let url: URL
    public let name: String
    public let isDirectory: Bool
    public let sizeBytes: Int
    public var children: [WorkspaceFileItem]?

    public init(url: URL, isDirectory: Bool, sizeBytes: Int = 0, children: [WorkspaceFileItem]? = nil) {
        self.url = url
        self.name = url.lastPathComponent
        self.isDirectory = isDirectory
        self.sizeBytes = sizeBytes
        self.children = children
    }

    public var fileIcon: String {
        if isDirectory { return "folder.fill" }
        let ext = url.pathExtension.lowercased()
        switch ext {
        case "swift": return "swift"
        case "rs": return "gearshape.2"
        case "py": return "command"
        case "ts", "tsx", "js", "jsx": return "curlybraces"
        case "json", "toml", "yaml", "yml": return "doc.badge.gearshape"
        case "md", "txt": return "doc.plaintext"
        case "c", "cpp", "h", "hpp": return "chevron.left.forwardslash.chevron.right"
        case "sh", "zsh", "bash": return "terminal"
        case "html", "css": return "chevron.left.forwardslash.chevron.right"
        default: return "doc"
        }
    }

    public var iconColor: Color {
        if isDirectory { return .accentColor }
        let ext = url.pathExtension.lowercased()
        switch ext {
        case "swift": return .orange
        case "rs": return .red
        case "py": return .yellow
        case "ts", "tsx", "js", "jsx": return .cyan
        case "json", "toml", "yaml", "yml": return .mint
        case "md", "txt": return .secondary
        default: return .secondary
        }
    }
}

// MARK: - Workspace File Tree View

public struct WorkspaceFileTree: View {
    @Binding var rootDirectory: URL
    public var onSelectFile: (URL, String) -> Void

    @State private var items: [WorkspaceFileItem] = []
    @State private var gitStatuses: [String: GitFileStatus] = [:]
    @State private var currentBranch: String = ""
    @State private var searchFilter: String = ""
    @State private var expandedPaths: Set<String> = []
    @State private var isLoading: Bool = false
    @State private var selectedPath: String?
    @State private var scanTask: Task<Void, Never>?
    @State private var gitTask: Task<Void, Never>?

    @State private var isNewFilePresented: Bool = false
    @State private var isNewFolderPresented: Bool = false
    @State private var isRenamePresented: Bool = false
    @State private var renameTarget: WorkspaceFileItem?
    @State private var renameText: String = ""
    @State private var newFileName: String = ""
    @State private var newFolderName: String = ""

    private enum ModalFocusField: Hashable {
        case newFile
        case newFolder
        case rename
    }
    @FocusState private var focusField: ModalFocusField?

    public init(
        rootDirectory: Binding<URL>,
        onSelectFile: @escaping (URL, String) -> Void
    ) {
        self._rootDirectory = rootDirectory
        self.onSelectFile = onSelectFile
    }

    public var body: some View {
        VStack(spacing: 0) {
            // Header Bar
            HStack(spacing: 6) {
                Image(systemName: "folder.fill")
                    .font(.system(size: 11))
                    .foregroundColor(.accentColor)

                Text(rootDirectory.lastPathComponent)
                    .font(.system(size: 11.5, weight: .bold))
                    .lineLimit(1)

                if !currentBranch.isEmpty {
                    HStack(spacing: 3) {
                        Image(systemName: "arrow.triangle.branch")
                            .font(.system(size: 8))
                        Text(currentBranch)
                            .font(.system(size: 9, weight: .medium, design: .monospaced))
                    }
                    .foregroundColor(.secondary)
                    .padding(.horizontal, 5)
                    .padding(.vertical, 1.5)
                    .background(Capsule().fill(Color.white.opacity(0.08)))
                }

                Spacer()

                let dirtyCount = gitStatuses.values.filter { $0 == .modified || $0 == .untracked || $0 == .added }.count
                if dirtyCount > 0 {
                    Text("\(dirtyCount)")
                        .font(.system(size: 9, weight: .bold, design: .monospaced))
                        .foregroundColor(.orange)
                        .padding(.horizontal, 4.5)
                        .padding(.vertical, 1)
                        .background(Capsule().fill(Color.orange.opacity(0.18)))
                        .help("\(dirtyCount) uncommitted change(s)")
                }

                // New File Button
                Button {
                    newFileName = ""
                    isNewFilePresented = true
                    LiquidGlass.haptic(.alignment)
                } label: {
                    Image(systemName: "plus")
                        .font(.system(size: 10, weight: .semibold))
                        .foregroundColor(.secondary)
                        .frame(width: 18, height: 18)
                        .background(Circle().fill(Color.white.opacity(0.06)))
                }
                .buttonStyle(.plain)
                .help("New File in Workspace")

                // New Folder Button
                Button {
                    newFolderName = ""
                    isNewFolderPresented = true
                    LiquidGlass.haptic(.alignment)
                } label: {
                    Image(systemName: "folder.badge.plus")
                        .font(.system(size: 10, weight: .semibold))
                        .foregroundColor(.secondary)
                        .frame(width: 18, height: 18)
                        .background(Circle().fill(Color.white.opacity(0.06)))
                }
                .buttonStyle(.plain)
                .help("New Folder in Workspace")

                // Change Root Folder Button
                Button {
                    chooseFolder()
                } label: {
                    Image(systemName: "ellipsis")
                        .font(.system(size: 10, weight: .semibold))
                        .foregroundColor(.secondary)
                        .frame(width: 18, height: 18)
                        .background(Circle().fill(Color.white.opacity(0.06)))
                }
                .buttonStyle(.plain)
                .help("Change Workspace Root Folder")

                // Refresh Button
                Button {
                    scanWorkspace()
                } label: {
                    Image(systemName: "arrow.clockwise")
                        .font(.system(size: 10))
                        .foregroundColor(.secondary)
                        .frame(width: 18, height: 18)
                        .background(Circle().fill(Color.white.opacity(0.06)))
                }
                .buttonStyle(.plain)
                .help("Refresh File Tree & Git Status")
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 8)
            .background(.ultraThinMaterial)
            .overlay(Divider().opacity(0.25), alignment: .bottom)

            // Search Bar
            HStack(spacing: 6) {
                Image(systemName: "magnifyingglass")
                    .font(.system(size: 10))
                    .foregroundColor(.secondary)

                TextField("Filter files...", text: $searchFilter)
                    .font(.system(size: 11))
                    .textFieldStyle(.plain)

                if !searchFilter.isEmpty {
                    Button {
                        searchFilter = ""
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
                RoundedRectangle(cornerRadius: 6, style: .continuous)
                    .fill(Color.white.opacity(0.05))
                    .overlay(RoundedRectangle(cornerRadius: 6).strokeBorder(Color.white.opacity(0.10), lineWidth: 0.5))
            )
            .padding(.horizontal, 8)
            .padding(.vertical, 6)

            // Tree Scroll Area
            if isLoading {
                VStack(spacing: 8) {
                    Spacer()
                    ProgressView()
                        .controlSize(.small)
                    Text("Scanning files…")
                        .font(.system(size: 10.5))
                        .foregroundColor(.secondary)
                    Spacer()
                }
            } else if items.isEmpty {
                VStack(spacing: 6) {
                    Spacer()
                    Image(systemName: "folder.badge.questionmark")
                        .font(.system(size: 20))
                        .foregroundColor(.secondary.opacity(0.6))
                    Text("No code files found")
                        .font(.system(size: 11))
                        .foregroundColor(.secondary)
                    Button("Open Folder…") {
                        chooseFolder()
                    }
                    .font(.system(size: 10.5))
                    .buttonStyle(.plain)
                    .foregroundColor(.accentColor)
                    Spacer()
                }
            } else {
                ScrollView(.vertical, showsIndicators: true) {
                    LazyVStack(alignment: .leading, spacing: 1) {
                        let filtered = filteredItems(items)
                        ForEach(filtered) { item in
                            WorkspaceFileRowView(
                                item: item,
                                depth: 0,
                                rootUrl: rootDirectory,
                                expandedPaths: $expandedPaths,
                                selectedPath: $selectedPath,
                                gitStatuses: gitStatuses,
                                onSelectFile: onSelectFile,
                                onRefresh: { scanWorkspace() },
                                onRename: { target in
                                    renameTarget = target
                                    renameText = target.name
                                    isRenamePresented = true
                                    LiquidGlass.haptic(.alignment)
                                }
                            )
                        }
                    }
                    .padding(.horizontal, 6)
                    .padding(.vertical, 4)
                }
            }
        }
        .frame(minWidth: 190, idealWidth: 230, maxWidth: 300)
        .background(.ultraThinMaterial)
        .onAppear {
            scanWorkspace()
        }
        .onChange(of: rootDirectory) { _ in
            scanWorkspace()
        }
        .sheet(isPresented: $isNewFilePresented) {
            newFileSheet
        }
        .sheet(isPresented: $isNewFolderPresented) {
            newFolderSheet
        }
        .sheet(isPresented: $isRenamePresented) {
            renameSheet
        }
    }

    // MARK: - New File & Folder Sheets

    private var newFileSheet: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack {
                Image(systemName: "doc.badge.plus")
                    .font(.system(size: 14, weight: .semibold))
                    .foregroundColor(.accentColor)
                Text("New File")
                    .font(.system(size: 13, weight: .bold))
                Spacer()
                Button {
                    LiquidGlass.haptic(.alignment)
                    isNewFilePresented = false
                } label: {
                    Image(systemName: "xmark.circle.fill")
                        .foregroundColor(.secondary)
                        .font(.system(size: 14))
                }
                .buttonStyle(.plain)
            }

            VStack(alignment: .leading, spacing: 6) {
                Text("Filename (e.g. main.rs, worker.swift, task.py):")
                    .font(.system(size: 11, weight: .medium))
                    .foregroundColor(.secondary)
                TextField("File name", text: $newFileName)
                    .textFieldStyle(.plain)
                    .font(.system(size: 12.5, design: .monospaced))
                    .liquidGlassTextField(cornerRadius: 8, isFocused: focusField == .newFile)
                    .focused($focusField, equals: .newFile)
                    .onSubmit {
                        createFile()
                    }
            }

            HStack(spacing: 10) {
                Spacer()
                Button("Cancel") {
                    LiquidGlass.haptic(.alignment)
                    isNewFilePresented = false
                }
                .keyboardShortcut(.cancelAction)
                .lacGlass()

                Button("Create File") {
                    createFile()
                }
                .keyboardShortcut(.defaultAction)
                .lacGlassProminent()
                .disabled(newFileName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
        }
        .padding(20)
        .frame(width: 340)
        .liquidGlassModalCard(cornerRadius: 18)
        .onAppear {
            focusField = .newFile
        }
    }

    private var newFolderSheet: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack {
                Image(systemName: "folder.badge.plus")
                    .font(.system(size: 14, weight: .semibold))
                    .foregroundColor(.accentColor)
                Text("New Folder")
                    .font(.system(size: 13, weight: .bold))
                Spacer()
                Button {
                    LiquidGlass.haptic(.alignment)
                    isNewFolderPresented = false
                } label: {
                    Image(systemName: "xmark.circle.fill")
                        .foregroundColor(.secondary)
                        .font(.system(size: 14))
                }
                .buttonStyle(.plain)
            }

            VStack(alignment: .leading, spacing: 6) {
                Text("Folder name:")
                    .font(.system(size: 11, weight: .medium))
                    .foregroundColor(.secondary)
                TextField("Folder name", text: $newFolderName)
                    .textFieldStyle(.plain)
                    .font(.system(size: 12.5))
                    .liquidGlassTextField(cornerRadius: 8, isFocused: focusField == .newFolder)
                    .focused($focusField, equals: .newFolder)
                    .onSubmit {
                        createFolder()
                    }
            }

            HStack(spacing: 10) {
                Spacer()
                Button("Cancel") {
                    LiquidGlass.haptic(.alignment)
                    isNewFolderPresented = false
                }
                .keyboardShortcut(.cancelAction)
                .lacGlass()

                Button("Create Folder") {
                    createFolder()
                }
                .keyboardShortcut(.defaultAction)
                .lacGlassProminent()
                .disabled(newFolderName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
        }
        .padding(20)
        .frame(width: 340)
        .liquidGlassModalCard(cornerRadius: 18)
        .onAppear {
            focusField = .newFolder
        }
    }

    private func createFile() {
        let trimmed = newFileName.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        let target = rootDirectory.appendingPathComponent(trimmed)
        let parentDir = target.deletingLastPathComponent()
        try? FileManager.default.createDirectory(at: parentDir, withIntermediateDirectories: true)
        if !FileManager.default.fileExists(atPath: target.path) {
            try? "".write(to: target, atomically: true, encoding: .utf8)
        }
        isNewFilePresented = false
        scanWorkspace()
        onSelectFile(target, "")
        LiquidGlass.haptic(.alignment)
    }

    private func createFolder() {
        let trimmed = newFolderName.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        let target = rootDirectory.appendingPathComponent(trimmed)
        try? FileManager.default.createDirectory(at: target, withIntermediateDirectories: true)
        isNewFolderPresented = false
        scanWorkspace()
        LiquidGlass.haptic(.alignment)
    }

    private var renameSheet: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack {
                Image(systemName: "pencil")
                    .font(.system(size: 14, weight: .semibold))
                    .foregroundColor(.accentColor)
                Text("Rename \(renameTarget?.isDirectory == true ? "Folder" : "File")")
                    .font(.system(size: 13, weight: .bold))
                Spacer()
                Button {
                    LiquidGlass.haptic(.alignment)
                    isRenamePresented = false
                } label: {
                    Image(systemName: "xmark.circle.fill")
                        .foregroundColor(.secondary)
                        .font(.system(size: 14))
                }
                .buttonStyle(.plain)
            }

            VStack(alignment: .leading, spacing: 6) {
                Text("New name:")
                    .font(.system(size: 11, weight: .medium))
                    .foregroundColor(.secondary)
                TextField("Name", text: $renameText)
                    .textFieldStyle(.plain)
                    .font(.system(size: 12.5, design: .monospaced))
                    .liquidGlassTextField(cornerRadius: 8, isFocused: focusField == .rename)
                    .focused($focusField, equals: .rename)
                    .onSubmit {
                        applyRename()
                    }
            }

            HStack(spacing: 10) {
                Spacer()
                Button("Cancel") {
                    LiquidGlass.haptic(.alignment)
                    isRenamePresented = false
                }
                .keyboardShortcut(.cancelAction)
                .lacGlass()

                Button("Apply Rename") {
                    applyRename()
                }
                .keyboardShortcut(.defaultAction)
                .lacGlassProminent()
                .disabled(renameText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
        }
        .padding(20)
        .frame(width: 340)
        .liquidGlassModalCard(cornerRadius: 18)
        .onAppear {
            focusField = .rename
        }
    }

    private func applyRename() {
        guard let target = renameTarget else { return }
        let trimmed = renameText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, trimmed != target.name else {
            isRenamePresented = false
            return
        }
        let dest = target.url.deletingLastPathComponent().appendingPathComponent(trimmed)
        try? FileManager.default.moveItem(at: target.url, to: dest)
        isRenamePresented = false
        renameTarget = nil
        scanWorkspace()
        LiquidGlass.haptic(.alignment)
    }
}

// MARK: - Recursive Tree Row

struct WorkspaceFileRowView: View {
    let item: WorkspaceFileItem
    let depth: Int
    let rootUrl: URL
    @Binding var expandedPaths: Set<String>
    @Binding var selectedPath: String?
    let gitStatuses: [String: GitFileStatus]
    let onSelectFile: (URL, String) -> Void
    let onRefresh: () -> Void
    let onRename: (WorkspaceFileItem) -> Void
    @State private var isHovered: Bool = false

    var body: some View {
        let isExpanded = expandedPaths.contains(item.id)
        let isSelected = selectedPath == item.id

        VStack(alignment: .leading, spacing: 1) {
            HStack(spacing: 5) {
                // Indentation
                if depth > 0 {
                    Spacer()
                        .frame(width: CGFloat(depth * 12))
                }

                // Folder chevron or file spacer
                if item.isDirectory {
                    Image(systemName: isExpanded ? "chevron.down" : "chevron.right")
                        .font(.system(size: 8, weight: .bold))
                        .foregroundColor(.secondary)
                        .frame(width: 10, height: 10)
                } else {
                    Spacer().frame(width: 10)
                }

                // Icon
                Image(systemName: item.fileIcon)
                    .font(.system(size: 10.5))
                    .foregroundColor(item.iconColor)
                    .frame(width: 14)

                // Filename
                Text(item.name)
                    .font(.system(size: 11, design: .monospaced))
                    .foregroundColor(isSelected ? .accentColor : .primary.opacity(0.9))
                    .lineLimit(1)

                Spacer()

                // Git status badge (M, U, A, D)
                if let status = gitStatuses[item.id] {
                    Text(status.badgeLabel)
                        .font(.system(size: 8.5, weight: .bold, design: .monospaced))
                        .foregroundColor(status.badgeColor)
                        .padding(.horizontal, 4)
                        .padding(.vertical, 1)
                        .background(status.badgeColor.opacity(0.20))
                        .clipShape(RoundedRectangle(cornerRadius: 3.5, style: .continuous))
                        .overlay(RoundedRectangle(cornerRadius: 3.5, style: .continuous).strokeBorder(status.badgeColor.opacity(0.35), lineWidth: 0.5))
                }
            }
            .padding(.vertical, 3.5)
            .padding(.horizontal, 6)
            .background(
                RoundedRectangle(cornerRadius: 6, style: .continuous)
                    .fill(
                        isSelected
                            ? Color.accentColor.opacity(0.20)
                            : (isHovered ? Color.white.opacity(0.06) : Color.clear)
                    )
                    .overlay(
                        RoundedRectangle(cornerRadius: 6, style: .continuous)
                            .strokeBorder(
                                isSelected
                                    ? Color.accentColor.opacity(0.35)
                                    : (isHovered ? Color.white.opacity(0.12) : Color.clear),
                                lineWidth: 0.5
                            )
                    )
            )
            .contentShape(Rectangle())
            .onHover { hovering in
                isHovered = hovering
            }
            .onTapGesture {
                if item.isDirectory {
                    if isExpanded {
                        expandedPaths.remove(item.id)
                    } else {
                        expandedPaths.insert(item.id)
                    }
                    LiquidGlass.haptic(.alignment)
                } else {
                    selectedPath = item.id
                    LiquidGlass.haptic(.alignment)
                    if let content = try? String(contentsOf: item.url, encoding: .utf8) {
                        onSelectFile(item.url, content)
                    }
                }
            }
            .contextMenu {
                if !item.isDirectory {
                    Button("Open in Editor") {
                        selectedPath = item.id
                        if let content = try? String(contentsOf: item.url, encoding: .utf8) {
                            onSelectFile(item.url, content)
                        }
                    }
                }
                Button("Reveal in Finder") {
                    NSWorkspace.shared.activateFileViewerSelecting([item.url])
                }
                Button("Rename...") {
                    onRename(item)
                }
                if !item.isDirectory {
                    Button("Duplicate") {
                        duplicateItem(item)
                    }
                }
                Button("Copy Path") {
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString(item.url.path, forType: .string)
                }
                Button("Copy Relative Path") {
                    let rel = item.url.path.replacingOccurrences(of: rootUrl.path + "/", with: "")
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString(rel, forType: .string)
                }
                Divider()
                Button("Move to Trash", role: .destructive) {
                    try? FileManager.default.trashItem(at: item.url, resultingItemURL: nil)
                    onRefresh()
                }
            }

            // Expanded children
            if item.isDirectory && isExpanded, let children = item.children {
                ForEach(children) { child in
                    WorkspaceFileRowView(
                        item: child,
                        depth: depth + 1,
                        rootUrl: rootUrl,
                        expandedPaths: $expandedPaths,
                        selectedPath: $selectedPath,
                        gitStatuses: gitStatuses,
                        onSelectFile: onSelectFile,
                        onRefresh: onRefresh,
                        onRename: onRename
                    )
                }
            }
        }
    }

    private func duplicateItem(_ item: WorkspaceFileItem) {
        let url = item.url
        let parent = url.deletingLastPathComponent()
        let ext = url.pathExtension
        let base = url.deletingPathExtension().lastPathComponent
        let copyName = ext.isEmpty ? "\(base) copy" : "\(base) copy.\(ext)"
        let dest = parent.appendingPathComponent(copyName)
        try? FileManager.default.copyItem(at: url, to: dest)
        onRefresh()
        LiquidGlass.haptic(.alignment)
    }
}

extension WorkspaceFileTree {

    // MARK: Filtering

    private func filteredItems(_ source: [WorkspaceFileItem]) -> [WorkspaceFileItem] {
        let q = searchFilter.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        guard !q.isEmpty else { return source }

        func filterItem(_ item: WorkspaceFileItem) -> WorkspaceFileItem? {
            if item.name.lowercased().contains(q) {
                return item
            }
            if item.isDirectory, let children = item.children {
                let matchedChildren = children.compactMap(filterItem)
                if !matchedChildren.isEmpty {
                    var copy = item
                    copy.children = matchedChildren
                    return copy
                }
            }
            return nil
        }

        return source.compactMap(filterItem)
    }

    // MARK: Folder Chooser

    private func chooseFolder() {
        let panel = NSOpenPanel()
        panel.canChooseFiles = false
        panel.canChooseDirectories = true
        panel.allowsMultipleSelection = false
        if panel.runModal() == .OK, let url = panel.url {
            rootDirectory = url
            scanWorkspace()
        }
    }

    // MARK: Git Status Scanner

    nonisolated public static func parseGitStatusOutput(_ output: String, rootUrl: URL) -> [String: GitFileStatus] {
        var map: [String: GitFileStatus] = [:]
        for line in output.components(separatedBy: "\n") {
            guard line.count >= 3 else { continue }
            let prefix = String(line.prefix(2))
            let pathPart = String(line.dropFirst(3)).trimmingCharacters(in: .whitespaces).trimmingCharacters(in: CharacterSet(charactersIn: "\""))
            guard !pathPart.isEmpty else { continue }
            let itemUrl = rootUrl.appendingPathComponent(pathPart).standardizedFileURL

            if prefix.contains("M") {
                map[itemUrl.path] = .modified
            } else if prefix.contains("?") {
                map[itemUrl.path] = .untracked
            } else if prefix.contains("A") {
                map[itemUrl.path] = .added
            } else if prefix.contains("D") {
                map[itemUrl.path] = .deleted
            }
        }
        return map
    }

    private func refreshGitStatus() {
        let root = rootDirectory
        gitTask?.cancel()
        gitTask = Task.detached(priority: .background) {
            let proc = Process()
            proc.executableURL = URL(fileURLWithPath: "/usr/bin/git")
            proc.arguments = ["status", "-s"]
            proc.currentDirectoryURL = root
            let pipe = Pipe()
            proc.standardOutput = pipe
            proc.standardError = Pipe()

            let bProc = Process()
            bProc.executableURL = URL(fileURLWithPath: "/usr/bin/git")
            bProc.arguments = ["branch", "--show-current"]
            bProc.currentDirectoryURL = root
            let bPipe = Pipe()
            bProc.standardOutput = bPipe
            bProc.standardError = Pipe()

            var parsedMap: [String: GitFileStatus] = [:]
            var branch = ""

            if (try? proc.run()) != nil {
                proc.waitUntilExit()
                let data = pipe.fileHandleForReading.readDataToEndOfFile()
                if !Task.isCancelled, let str = String(data: data, encoding: .utf8) {
                    parsedMap = Self.parseGitStatusOutput(str, rootUrl: root)
                }
            }

            if (try? bProc.run()) != nil {
                bProc.waitUntilExit()
                let bData = bPipe.fileHandleForReading.readDataToEndOfFile()
                if !Task.isCancelled, let bStr = String(data: bData, encoding: .utf8) {
                    branch = bStr.trimmingCharacters(in: .whitespacesAndNewlines)
                }
            }
            
            guard !Task.isCancelled else { return }

            await MainActor.run {
                self.gitStatuses = parsedMap
                self.currentBranch = branch
            }
        }
    }

    // MARK: Workspace Scanner

    private func scanWorkspace() {
        isLoading = true
        let url = rootDirectory
        refreshGitStatus()

        scanTask?.cancel()
        scanTask = Task.detached(priority: .userInitiated) {
            let scanned = Self.scanDirectory(url: url, maxDepth: 4)
            guard !Task.isCancelled else { return }
            await MainActor.run {
                self.items = scanned
                self.isLoading = false
            }
        }
    }

    nonisolated private static func scanDirectory(url: URL, maxDepth: Int) -> [WorkspaceFileItem] {
        guard maxDepth > 0, !Task.isCancelled else { return [] }
        let fm = FileManager.default
        let ignoredFolders: Set<String> = [
            ".git", ".build", "target", "node_modules", "DerivedData",
            ".DS_Store", ".idea", ".vscode", "tmp", "coverage"
        ]

        guard let contents = try? fm.contentsOfDirectory(
            at: url,
            includingPropertiesForKeys: [.isDirectoryKey, .fileSizeKey],
            options: [.skipsHiddenFiles]
        ) else { return [] }

        var dirs: [WorkspaceFileItem] = []
        var files: [WorkspaceFileItem] = []

        for itemUrl in contents {
            let name = itemUrl.lastPathComponent
            if ignoredFolders.contains(name) { continue }

            let isDir = (try? itemUrl.resourceValues(forKeys: [.isDirectoryKey]).isDirectory) ?? false
            if isDir {
                let children = scanDirectory(url: itemUrl, maxDepth: maxDepth - 1)
                dirs.append(WorkspaceFileItem(url: itemUrl, isDirectory: true, children: children))
            } else {
                let size = (try? itemUrl.resourceValues(forKeys: Set([URLResourceKey.fileSizeKey])).fileSize) ?? 0
                files.append(WorkspaceFileItem(url: itemUrl, isDirectory: false, sizeBytes: size))
            }
        }

        dirs.sort { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }
        files.sort { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }

        return dirs + files
    }

    /// Recursively scans and collects all non-ignored files into a flat list for Quick Open (⌘P).
    nonisolated public static func scanFlatFiles(url: URL, maxDepth: Int = 5) -> [WorkspaceFileItem] {
        guard maxDepth > 0 else { return [] }
        let fm = FileManager.default
        let ignoredFolders: Set<String> = [
            ".git", ".build", "target", "node_modules", "DerivedData",
            ".DS_Store", ".idea", ".vscode", "tmp", "coverage"
        ]

        guard let contents = try? fm.contentsOfDirectory(
            at: url,
            includingPropertiesForKeys: [.isDirectoryKey, .fileSizeKey],
            options: [.skipsHiddenFiles]
        ) else { return [] }

        var results: [WorkspaceFileItem] = []

        for itemUrl in contents {
            let name = itemUrl.lastPathComponent
            if ignoredFolders.contains(name) { continue }

            let isDir = (try? itemUrl.resourceValues(forKeys: [.isDirectoryKey]).isDirectory) ?? false
            if isDir {
                results.append(contentsOf: scanFlatFiles(url: itemUrl, maxDepth: maxDepth - 1))
            } else {
                let size = (try? itemUrl.resourceValues(forKeys: Set([URLResourceKey.fileSizeKey])).fileSize) ?? 0
                results.append(WorkspaceFileItem(url: itemUrl, isDirectory: false, sizeBytes: size))
            }
        }
        return results.sorted { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }
    }
}
