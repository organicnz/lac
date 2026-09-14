import AppKit
import Foundation
import SwiftUI

// MARK: - Quick File Open Modal (⌘P)

public struct QuickFileOpenView: View {
    @Binding var isPresented: Bool
    let rootUrl: URL
    let onSelect: (URL, String) -> Void

    @State private var query: String = ""
    @State private var selectedIndex: Int = 0
    @State private var allFiles: [WorkspaceFileItem] = []
    @State private var isLoading: Bool = true
    @FocusState private var isSearchFocused: Bool

    public init(
        isPresented: Binding<Bool>,
        rootUrl: URL,
        onSelect: @escaping (URL, String) -> Void
    ) {
        self._isPresented = isPresented
        self.rootUrl = rootUrl
        self.onSelect = onSelect
    }

    private var filteredFiles: [WorkspaceFileItem] {
        let q = query.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        if q.isEmpty {
            return Array(allFiles.prefix(40))
        }
        let matching = allFiles.filter { item in
            item.name.lowercased().contains(q) ||
            item.url.path.lowercased().contains(q)
        }
        // Prioritize exact prefix match on filename
        let sorted = matching.sorted { a, b in
            let aPref = a.name.lowercased().hasPrefix(q)
            let bPref = b.name.lowercased().hasPrefix(q)
            if aPref && !bPref { return true }
            if !aPref && bPref { return false }
            return a.name.localizedCaseInsensitiveCompare(b.name) == .orderedAscending
        }
        return Array(sorted.prefix(50))
    }

    public var body: some View {
        ZStack {
            // Backdrop
            Color.black.opacity(0.40)
                .ignoresSafeArea()
                .onTapGesture {
                    isPresented = false
                }

            // Floating Quick Open Card
            VStack(spacing: 0) {
                // Search Input Header
                HStack(spacing: 10) {
                    Image(systemName: "magnifyingglass")
                        .font(.system(size: 14, weight: .semibold))
                        .foregroundColor(.accentColor)

                    TextField("Quick Open (⌘P) — type filename or path…", text: $query)
                        .textFieldStyle(.plain)
                        .font(.system(size: 13, weight: .medium))
                        .focused($isSearchFocused)
                        .onSubmit {
                            selectCurrent()
                        }

                    if !query.isEmpty {
                        Button {
                            query = ""
                        } label: {
                            Image(systemName: "xmark.circle.fill")
                                .font(.system(size: 12))
                                .foregroundColor(.secondary)
                        }
                        .buttonStyle(.plain)
                    }

                    Text("ESC")
                        .font(.system(size: 9.5, weight: .bold, design: .monospaced))
                        .foregroundColor(.secondary)
                        .padding(.horizontal, 5)
                        .padding(.vertical, 2)
                        .background(Capsule().fill(Color.white.opacity(0.08)))
                }
                .padding(.horizontal, 16)
                .padding(.vertical, 14)
                .background(Color.white.opacity(0.05))
                .overlay(Divider().opacity(0.2), alignment: .bottom)

                // Results Container
                if isLoading {
                    HStack(spacing: 8) {
                        ProgressView()
                            .scaleEffect(0.8)
                        Text("Indexing workspace files…")
                            .font(.system(size: 11))
                            .foregroundColor(.secondary)
                    }
                    .frame(maxWidth: .infinity, minHeight: 180)
                } else if filteredFiles.isEmpty {
                    VStack(spacing: 6) {
                        Image(systemName: "doc.text.magnifyingglass")
                            .font(.system(size: 26))
                            .foregroundColor(.secondary.opacity(0.5))
                            .padding(.bottom, 2)
                        Text("No matching files found")
                            .font(.system(size: 12, weight: .medium))
                            .foregroundColor(.secondary)
                        Text("Try searching by extension (e.g. .rs, .swift)")
                            .font(.system(size: 10))
                            .foregroundColor(.secondary.opacity(0.7))
                    }
                    .frame(maxWidth: .infinity, minHeight: 180)
                } else {
                    ScrollViewReader { proxy in
                        ScrollView(.vertical, showsIndicators: true) {
                            LazyVStack(spacing: 2) {
                                ForEach(Array(filteredFiles.enumerated()), id: \.element.id) { index, item in
                                    let isSelected = index == selectedIndex
                                    fileRow(item: item, isSelected: isSelected)
                                        .id(index)
                                        .onTapGesture {
                                            openItem(item)
                                        }
                                }
                            }
                            .padding(.horizontal, 8)
                            .padding(.vertical, 6)
                        }
                        .frame(maxHeight: 320)
                        .onChange(of: selectedIndex) { idx in
                            proxy.scrollTo(idx, anchor: .center)
                        }
                    }
                }

                // Footer Info Bar
                HStack {
                    Text("\(filteredFiles.count) file\(filteredFiles.count == 1 ? "" : "s")")
                        .font(.system(size: 10, design: .monospaced))
                        .foregroundColor(.secondary)

                    Spacer()

                    HStack(spacing: 12) {
                        HStack(spacing: 4) {
                            Text("↵")
                                .font(.system(size: 10, weight: .bold))
                                .foregroundColor(.secondary)
                            Text("Open")
                                .font(.system(size: 10))
                                .foregroundColor(.secondary)
                        }
                        HStack(spacing: 4) {
                            Text("Esc")
                                .font(.system(size: 10, weight: .bold))
                                .foregroundColor(.secondary)
                            Text("Close")
                                .font(.system(size: 10))
                                .foregroundColor(.secondary)
                        }
                    }
                }
                .padding(.horizontal, 14)
                .padding(.vertical, 8)
                .background(Color.black.opacity(0.22))
                .overlay(Divider().opacity(0.18), alignment: .top)
            }
            .frame(width: 580)
            .background(.ultraThinMaterial)
            .clipShape(RoundedRectangle(cornerRadius: 14, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: 14, style: .continuous)
                    .strokeBorder(Color.white.opacity(0.18), lineWidth: 1)
            )
            .shadow(color: Color.black.opacity(0.45), radius: 30, y: 14)
        }
        .onAppear {
            scanWorkspaceFiles()
            isSearchFocused = true
        }
        .onChange(of: query) { _ in
            selectedIndex = 0
        }
        .background {
            Group {
                Button("") {
                    isPresented = false
                }
                .keyboardShortcut(.escape, modifiers: [])

                Button("") {
                    if selectedIndex > 0 {
                        selectedIndex -= 1
                    }
                }
                .keyboardShortcut(.upArrow, modifiers: [])

                Button("") {
                    if selectedIndex < filteredFiles.count - 1 {
                        selectedIndex += 1
                    }
                }
                .keyboardShortcut(.downArrow, modifiers: [])
            }
            .opacity(0)
            .allowsHitTesting(false)
        }
    }

    // MARK: - File Row

    private func fileRow(item: WorkspaceFileItem, isSelected: Bool) -> some View {
        HStack(spacing: 10) {
            Image(systemName: item.fileIcon)
                .font(.system(size: 13, weight: .semibold))
                .foregroundColor(item.iconColor)
                .frame(width: 18)

            VStack(alignment: .leading, spacing: 2) {
                Text(item.name)
                    .font(.system(size: 12, weight: .semibold))
                    .foregroundColor(isSelected ? .white : .primary)

                let rel = item.url.path.replacingOccurrences(of: rootUrl.path + "/", with: "")
                Text(rel)
                    .font(.system(size: 10, design: .monospaced))
                    .foregroundColor(.secondary.opacity(0.8))
                    .lineLimit(1)
            }

            Spacer()

            if item.sizeBytes > 0 {
                Text(formatFileSize(item.sizeBytes))
                    .font(.system(size: 9.5, design: .monospaced))
                    .foregroundColor(.secondary.opacity(0.6))
            }
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 7)
        .background(
            RoundedRectangle(cornerRadius: 8, style: .continuous)
                .fill(isSelected ? Color.accentColor.opacity(0.25) : Color.clear)
        )
        .contentShape(Rectangle())
    }

    private func selectCurrent() {
        let list = filteredFiles
        guard !list.isEmpty else { return }
        let idx = min(max(0, selectedIndex), list.count - 1)
        openItem(list[idx])
    }

    private func openItem(_ item: WorkspaceFileItem) {
        if let content = try? String(contentsOf: item.url, encoding: .utf8) {
            onSelect(item.url, content)
            isPresented = false
            LiquidGlass.haptic(.alignment)
        }
    }

    private func scanWorkspaceFiles() {
        isLoading = true
        let root = rootUrl
        Task.detached(priority: .userInitiated) {
            let files = WorkspaceFileTree.scanFlatFiles(url: root, maxDepth: 5)
            await MainActor.run {
                self.allFiles = files
                self.isLoading = false
            }
        }
    }

    private func formatFileSize(_ bytes: Int) -> String {
        if bytes < 1024 {
            return "\(bytes) B"
        } else if bytes < 1024 * 1024 {
            let kb = Double(bytes) / 1024.0
            return String(format: "%.1f KB", kb)
        } else {
            let mb = Double(bytes) / (1024.0 * 1024.0)
            return String(format: "%.1f MB", mb)
        }
    }
}
