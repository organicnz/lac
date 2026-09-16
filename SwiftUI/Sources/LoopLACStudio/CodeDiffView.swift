import AppKit
import SwiftUI

// MARK: - Line Diff Types

public enum DiffLineType: Equatable, Sendable {
    case added
    case removed
    case unchanged
}

public struct DiffLine: Identifiable, Equatable, Sendable {
    public let id = UUID()
    public let type: DiffLineType
    public let text: String
    public let oldLineNumber: Int?
    public let newLineNumber: Int?

    public init(type: DiffLineType, text: String, oldLineNumber: Int?, newLineNumber: Int?) {
        self.type = type
        self.text = text
        self.oldLineNumber = oldLineNumber
        self.newLineNumber = newLineNumber
    }
}

// MARK: - Diff Hunk (Chunk-level selective diffing)

public enum HunkState: String, Equatable, Sendable {
    case pending = "Pending"
    case accepted = "Accepted"
    case rejected = "Rejected"
}

public struct DiffHunk: Identifiable, Equatable, Sendable {
    public let id = UUID()
    public let hunkIndex: Int
    public let oldStartLine: Int
    public let oldLineCount: Int
    public let newStartLine: Int
    public let newLineCount: Int
    public var lines: [DiffLine]
    public var state: HunkState

    public init(
        hunkIndex: Int,
        oldStartLine: Int,
        oldLineCount: Int,
        newStartLine: Int,
        newLineCount: Int,
        lines: [DiffLine],
        state: HunkState = .pending
    ) {
        self.hunkIndex = hunkIndex
        self.oldStartLine = oldStartLine
        self.oldLineCount = oldLineCount
        self.newStartLine = newStartLine
        self.newLineCount = newLineCount
        self.lines = lines
        self.state = state
    }

    public var header: String {
        "@@ -\(oldStartLine),\(oldLineCount) +\(newStartLine),\(newLineCount) @@"
    }

    public var addedCount: Int {
        lines.filter { $0.type == .added }.count
    }

    public var removedCount: Int {
        lines.filter { $0.type == .removed }.count
    }
}

// MARK: - Longest Common Subsequence (LCS) Line Diff Algorithm

public func computeLineDiff(original: String, modified: String) -> [DiffLine] {
    let origLines = original.split(separator: "\n", omittingEmptySubsequences: false).map(String.init)
    let modLines = modified.split(separator: "\n", omittingEmptySubsequences: false).map(String.init)

    let n = origLines.count
    let m = modLines.count

    // Fast path: identical strings
    if original == modified {
        return origLines.enumerated().map { idx, line in
            DiffLine(type: .unchanged, text: line, oldLineNumber: idx + 1, newLineNumber: idx + 1)
        }
    }

    // Guard path: the DP table below is O(n·m) Ints — a 5k-line assistant
    // output is 25M entries on the calling thread (freeze/OOM). Degrade to
    // a linear prefix/suffix trim with a changed middle instead.
    if n * m > 4_000_000 {
        return simpleLineDiff(origLines: origLines, modLines: modLines)
    }

    // Standard dynamic programming LCS table
    var dp = Array(repeating: Array(repeating: 0, count: m + 1), count: n + 1)
    for i in 0..<n {
        for j in 0..<m {
            if origLines[i] == modLines[j] {
                dp[i + 1][j + 1] = dp[i][j] + 1
            } else {
                dp[i + 1][j + 1] = max(dp[i + 1][j], dp[i][j + 1])
            }
        }
    }

    // Backtrack to build unified diff lines
    var diff: [DiffLine] = []
    var i = n
    var j = m

    while i > 0 || j > 0 {
        if i > 0 && j > 0 && origLines[i - 1] == modLines[j - 1] {
            diff.append(DiffLine(type: .unchanged, text: origLines[i - 1], oldLineNumber: i, newLineNumber: j))
            i -= 1
            j -= 1
        } else if j > 0 && (i == 0 || dp[i][j - 1] >= dp[i - 1][j]) {
            diff.append(DiffLine(type: .added, text: modLines[j - 1], oldLineNumber: nil, newLineNumber: j))
            j -= 1
        } else if i > 0 && (j == 0 || dp[i][j - 1] < dp[i - 1][j]) {
            diff.append(DiffLine(type: .removed, text: origLines[i - 1], oldLineNumber: i, newLineNumber: nil))
            i -= 1
        }
    }

    return diff.reversed()
}

/// Linear fallback for `computeLineDiff` when the O(n·m) DP table would
/// explode: longest common prefix + suffix stay `unchanged`, the middle
/// becomes removed+added. O(n+m) time, O(1) extra space.
public func simpleLineDiff(origLines: [String], modLines: [String]) -> [DiffLine] {
    var prefix = 0
    while prefix < origLines.count && prefix < modLines.count
        && origLines[prefix] == modLines[prefix] {
        prefix += 1
    }
    var suffix = 0
    while suffix < origLines.count - prefix && suffix < modLines.count - prefix
        && origLines[origLines.count - 1 - suffix] == modLines[modLines.count - 1 - suffix] {
        suffix += 1
    }
    var out: [DiffLine] = []
    out.reserveCapacity(origLines.count + modLines.count)
    for i in 0..<prefix {
        out.append(DiffLine(type: .unchanged, text: origLines[i], oldLineNumber: i + 1, newLineNumber: i + 1))
    }
    for i in prefix..<(origLines.count - suffix) {
        out.append(DiffLine(type: .removed, text: origLines[i], oldLineNumber: i + 1, newLineNumber: nil))
    }
    for j in prefix..<(modLines.count - suffix) {
        out.append(DiffLine(type: .added, text: modLines[j], oldLineNumber: nil, newLineNumber: j + 1))
    }
    for k in 0..<suffix {
        let i = origLines.count - suffix + k
        let j = modLines.count - suffix + k
        out.append(DiffLine(type: .unchanged, text: origLines[i], oldLineNumber: i + 1, newLineNumber: j + 1))
    }
    return out
}

// MARK: - Partition Diff into Hunks

public func partitionIntoHunks(diffLines: [DiffLine], contextRadius: Int = 3) -> [DiffHunk] {
    var changeIndices: [Int] = []
    for (idx, line) in diffLines.enumerated() {
        if line.type != .unchanged {
            changeIndices.append(idx)
        }
    }

    guard !changeIndices.isEmpty else { return [] }

    // Cluster changes that are within 2 * contextRadius
    var clusters: [[Int]] = []
    var currentCluster: [Int] = [changeIndices[0]]

    for i in 1..<changeIndices.count {
        let prev = changeIndices[i - 1]
        let curr = changeIndices[i]
        if curr - prev <= (contextRadius * 2) {
            currentCluster.append(curr)
        } else {
            clusters.append(currentCluster)
            currentCluster = [curr]
        }
    }
    clusters.append(currentCluster)

    // Build each hunk with context lines
    var hunks: [DiffHunk] = []
    for (hunkIdx, cluster) in clusters.enumerated() {
        guard let first = cluster.first, let last = cluster.last else { continue }
        let startIdx = max(0, first - contextRadius)
        let endIdx = min(diffLines.count - 1, last + contextRadius)

        let slice = Array(diffLines[startIdx...endIdx])

        let oldLines = slice.filter { $0.type != .added }
        let newLines = slice.filter { $0.type != .removed }

        let oldStart = oldLines.compactMap(\.oldLineNumber).min() ?? 1
        let oldCount = oldLines.count

        let newStart = newLines.compactMap(\.newLineNumber).min() ?? 1
        let newCount = newLines.count

        hunks.append(
            DiffHunk(
                hunkIndex: hunkIdx + 1,
                oldStartLine: oldStart,
                oldLineCount: oldCount,
                newStartLine: newStart,
                newLineCount: newCount,
                lines: slice,
                state: .pending
            )
        )
    }

    return hunks
}

// MARK: - Synthesize Code from Hunk States

public func synthesizeCodeFromHunks(
    diffLines: [DiffLine],
    hunks: [DiffHunk]
) -> String {
    // If all hunks are accepted or pending without selective rejects, modifiedCode is returned
    let acceptedHunkIndices = Set(hunks.filter { $0.state == .accepted }.map(\.hunkIndex))
    let rejectedHunkIndices = Set(hunks.filter { $0.state == .rejected }.map(\.hunkIndex))

    if rejectedHunkIndices.isEmpty {
        // Return full modified lines
        let mod = diffLines.filter { $0.type != .removed }.map(\.text)
        return mod.joined(separator: "\n")
    }

    if acceptedHunkIndices.isEmpty && !rejectedHunkIndices.isEmpty {
        // Everything rejected -> return original lines
        let orig = diffLines.filter { $0.type != .added }.map(\.text)
        return orig.joined(separator: "\n")
    }

    // Selective: reconstruct line by line
    var resultLines: [String] = []
    for line in diffLines {
        // Find if this line belongs to a rejected hunk
        let inRejected = hunks.contains { h in
            rejectedHunkIndices.contains(h.hunkIndex) && h.lines.contains { $0.id == line.id }
        }

        if inRejected {
            // For rejected hunks: keep original (unchanged + removed), drop added
            if line.type != .added {
                resultLines.append(line.text)
            }
        } else {
            // For accepted/pending hunks: keep modified (unchanged + added), drop removed
            if line.type != .removed {
                resultLines.append(line.text)
            }
        }
    }

    return resultLines.joined(separator: "\n")
}

// MARK: - Code Diff View

public struct CodeDiffView: View {
    public let originalCode: String
    public let modifiedCode: String
    public var onAccept: (() -> Void)?
    public var onApplyCode: ((String) -> Void)?

    @State private var diffLines: [DiffLine] = []
    @State private var hunks: [DiffHunk] = []
    @State private var viewMode: DiffDisplayMode = .hunks
    @State private var diffTask: Task<Void, Never>?

    public enum DiffDisplayMode: String, CaseIterable, Identifiable {
        case hunks = "Hunks (Chunk-by-Chunk)"
        case unified = "Unified Diff"
        public var id: String { rawValue }
    }

    public init(
        originalCode: String,
        modifiedCode: String,
        onAccept: (() -> Void)? = nil,
        onApplyCode: ((String) -> Void)? = nil
    ) {
        self.originalCode = originalCode
        self.modifiedCode = modifiedCode
        self.onAccept = onAccept
        self.onApplyCode = onApplyCode
    }

    public var body: some View {
        VStack(spacing: 0) {
            // Diff Stats & Actions Header
            HStack {
                let addedCount = diffLines.filter { $0.type == .added }.count
                let removedCount = diffLines.filter { $0.type == .removed }.count

                HStack(spacing: 8) {
                    HStack(spacing: 4) {
                        Text("+\(addedCount)")
                            .font(.system(size: 11, weight: .bold, design: .monospaced))
                            .foregroundColor(.green)
                        Text("added")
                            .font(.system(size: 10))
                            .foregroundColor(.secondary)
                    }
                    Text("•")
                        .foregroundColor(.secondary.opacity(0.4))
                    HStack(spacing: 4) {
                        Text("-\(removedCount)")
                            .font(.system(size: 11, weight: .bold, design: .monospaced))
                            .foregroundColor(.red)
                        Text("removed")
                            .font(.system(size: 10))
                            .foregroundColor(.secondary)
                    }
                    if !hunks.isEmpty {
                        Text("•")
                            .foregroundColor(.secondary.opacity(0.4))
                        Text("\(hunks.count) chunk\(hunks.count == 1 ? "" : "s")")
                            .font(.system(size: 10, design: .monospaced))
                            .foregroundColor(.secondary)
                    }
                }

                Spacer()

                // Display Mode Picker
                if !hunks.isEmpty {
                    Picker("Mode", selection: $viewMode) {
                        ForEach(DiffDisplayMode.allCases) { mode in
                            Text(mode == .hunks ? "Chunk Mode" : "Unified").tag(mode)
                        }
                    }
                    .pickerStyle(.segmented)
                    .frame(width: 170)
                }

                // Global Actions
                HStack(spacing: 6) {
                    Button {
                        rejectAllHunks()
                    } label: {
                        HStack(spacing: 4) {
                            Image(systemName: "xmark.circle")
                                .font(.system(size: 10))
                            Text("Reject All")
                                .font(.system(size: 10.5))
                        }
                    }
                    .controlSize(.small)
                    .lacGlass()

                    Button {
                        acceptAllHunks()
                    } label: {
                        HStack(spacing: 4) {
                            Image(systemName: "checkmark.circle.fill")
                                .font(.system(size: 10))
                            Text("Accept All")
                                .font(.system(size: 10.5, weight: .semibold))
                        }
                    }
                    .controlSize(.small)
                    .lacGlassProminent()
                }
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 8)
            .background(Color.black.opacity(0.20))
            .overlay(Divider().opacity(0.25), alignment: .bottom)

            // Content Area
            if diffLines.isEmpty || originalCode == modifiedCode {
                VStack(spacing: 8) {
                    Spacer()
                    Image(systemName: "checkmark.seal")
                        .font(.system(size: 24))
                        .foregroundColor(.green.opacity(0.7))
                    Text("No differences detected — code is identical")
                        .font(.system(size: 12))
                        .foregroundColor(.secondary)
                    Spacer()
                }
            } else if viewMode == .hunks && !hunks.isEmpty {
                // Chunk-by-chunk hunk list
                ScrollView([.horizontal, .vertical]) {
                    VStack(alignment: .leading, spacing: 12) {
                        ForEach(hunks) { hunk in
                            hunkCard(hunk)
                        }
                    }
                    .padding(12)
                }
                .background(Color.black.opacity(0.35))
            } else {
                // Unified linear diff
                ScrollView([.horizontal, .vertical]) {
                    VStack(alignment: .leading, spacing: 0) {
                        ForEach(diffLines) { line in
                            diffLineRow(line)
                        }
                    }
                    .padding(.vertical, 8)
                }
                .background(Color.black.opacity(0.35))
            }
        }
        .onAppear {
            recalculateDiff()
        }
        .onDisappear {
            diffTask?.cancel()
        }
        .onChange(of: modifiedCode) { _ in
            recalculateDiff()
        }
        .onChange(of: originalCode) { _ in
            recalculateDiff()
        }
    }

    // MARK: Hunk Card

    private func hunkCard(_ hunk: DiffHunk) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            // Hunk Header with chunk-level controls
            HStack {
                HStack(spacing: 6) {
                    Text("Chunk #\(hunk.hunkIndex)")
                        .font(.system(size: 11, weight: .bold))
                        .foregroundColor(.primary)

                    Text(hunk.header)
                        .font(.system(size: 10, design: .monospaced))
                        .foregroundColor(.secondary)
                }

                Spacer()

                // State badge
                switch hunk.state {
                case .pending:
                    EmptyView()
                case .accepted:
                    HStack(spacing: 3) {
                        Image(systemName: "checkmark")
                            .font(.system(size: 8, weight: .bold))
                        Text("Accepted")
                            .font(.system(size: 9.5, weight: .medium))
                    }
                    .foregroundColor(.green)
                    .padding(.horizontal, 6)
                    .padding(.vertical, 2)
                    .background(Capsule().fill(Color.green.opacity(0.18)))
                case .rejected:
                    HStack(spacing: 3) {
                        Image(systemName: "xmark")
                            .font(.system(size: 8, weight: .bold))
                        Text("Rejected")
                            .font(.system(size: 9.5, weight: .medium))
                    }
                    .foregroundColor(.red)
                    .padding(.horizontal, 6)
                    .padding(.vertical, 2)
                    .background(Capsule().fill(Color.red.opacity(0.18)))
                }

                // Chunk actions
                HStack(spacing: 6) {
                    Button {
                        toggleHunk(hunkIndex: hunk.hunkIndex, state: .rejected)
                    } label: {
                        HStack(spacing: 3) {
                            Image(systemName: "xmark")
                                .font(.system(size: 9, weight: .bold))
                            Text("Reject")
                                .font(.system(size: 10))
                        }
                        .foregroundColor(hunk.state == .rejected ? .red : .secondary)
                        .padding(.horizontal, 7)
                        .padding(.vertical, 3)
                        .background(
                            Capsule().fill(hunk.state == .rejected ? Color.red.opacity(0.15) : Color.white.opacity(0.06))
                        )
                    }
                    .buttonStyle(.plain)
                    .help("Reject this chunk and keep original lines")

                    Button {
                        toggleHunk(hunkIndex: hunk.hunkIndex, state: .accepted)
                    } label: {
                        HStack(spacing: 3) {
                            Image(systemName: "checkmark")
                                .font(.system(size: 9, weight: .bold))
                            Text("Accept")
                                .font(.system(size: 10, weight: .semibold))
                        }
                        .foregroundColor(hunk.state == .accepted ? .green : .accentColor)
                        .padding(.horizontal, 8)
                        .padding(.vertical, 3)
                        .background(
                            Capsule().fill(hunk.state == .accepted ? Color.green.opacity(0.20) : Color.accentColor.opacity(0.15))
                        )
                    }
                    .buttonStyle(.plain)
                    .help("Accept this chunk and apply changes")
                }
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 6)
            .background(Color.white.opacity(0.06))
            .overlay(Divider().opacity(0.2), alignment: .bottom)

            // Hunk Lines
            VStack(alignment: .leading, spacing: 0) {
                ForEach(hunk.lines) { line in
                    diffLineRow(line)
                }
            }
        }
        .background(Color.black.opacity(0.40))
        .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 8, style: .continuous)
                .strokeBorder(
                    hunk.state == .accepted ? Color.green.opacity(0.35) :
                    (hunk.state == .rejected ? Color.red.opacity(0.35) : Color.white.opacity(0.10)),
                    lineWidth: 1
                )
        )
    }

    // MARK: Diff Line Row

    private func diffLineRow(_ line: DiffLine) -> some View {
        HStack(spacing: 0) {
            // Line numbers (old / new)
            HStack(spacing: 4) {
                Text(line.oldLineNumber.map { "\($0)" } ?? " ")
                    .font(.system(size: 10.5, design: .monospaced))
                    .foregroundColor(.secondary.opacity(0.6))
                    .frame(width: 28, alignment: .trailing)

                Text(line.newLineNumber.map { "\($0)" } ?? " ")
                    .font(.system(size: 10.5, design: .monospaced))
                    .foregroundColor(.secondary.opacity(0.6))
                    .frame(width: 28, alignment: .trailing)
            }
            .padding(.trailing, 8)

            // Prefix symbol (+ / - / space)
            Text(line.type == .added ? "+" : (line.type == .removed ? "-" : " "))
                .font(.system(size: 12, weight: .bold, design: .monospaced))
                .foregroundColor(lineColor(line.type))
                .frame(width: 14, alignment: .center)

            // Line Text
            Text(line.text.isEmpty ? " " : line.text)
                .font(.system(size: 12, design: .monospaced))
                .foregroundColor(lineTextColor(line.type))
                .textSelection(.enabled)

            Spacer()
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 1.5)
        .background(lineBackground(line.type))
    }

    private func lineColor(_ type: DiffLineType) -> Color {
        switch type {
        case .added: return .green
        case .removed: return .red
        case .unchanged: return .secondary.opacity(0.5)
        }
    }

    private func lineTextColor(_ type: DiffLineType) -> Color {
        switch type {
        case .added: return .green.opacity(0.95)
        case .removed: return .red.opacity(0.95)
        case .unchanged: return .primary
        }
    }

    private func lineBackground(_ type: DiffLineType) -> Color {
        switch type {
        case .added: return Color.green.opacity(0.12)
        case .removed: return Color.red.opacity(0.12)
        case .unchanged: return Color.clear
        }
    }

    // MARK: Actions & Synthesis

    // Diffing runs detached (never on MainActor) with a stale-guard:
    // a newer keystroke cancels the in-flight computation and its late
    // completion is discarded instead of overwriting fresher results.
    private func recalculateDiff() {
        diffTask?.cancel()
        let orig = originalCode, mod = modifiedCode
        diffTask = Task.detached(priority: .userInitiated) {
            let lines = computeLineDiff(original: orig, modified: mod)
            let computed = partitionIntoHunks(diffLines: lines)
            await MainActor.run {
                guard orig == self.originalCode, mod == self.modifiedCode else { return }
                guard !Task.isCancelled else { return }
                self.diffLines = lines
                self.hunks = computed
            }
        }
    }

    private func toggleHunk(hunkIndex: Int, state: HunkState) {
        LiquidGlass.haptic(.alignment)
        if let idx = hunks.firstIndex(where: { $0.hunkIndex == hunkIndex }) {
            hunks[idx].state = (hunks[idx].state == state) ? .pending : state
            applyCurrentHunkState()
        }
    }

    private func acceptAllHunks() {
        LiquidGlass.haptic(.alignment)
        for i in hunks.indices {
            hunks[i].state = .accepted
        }
        if let onAccept {
            onAccept()
        } else if let onApplyCode {
            onApplyCode(modifiedCode)
        }
    }

    private func rejectAllHunks() {
        LiquidGlass.haptic(.alignment)
        for i in hunks.indices {
            hunks[i].state = .rejected
        }
        if let onApplyCode {
            onApplyCode(originalCode)
        }
    }

    private func applyCurrentHunkState() {
        guard let onApplyCode else { return }
        let synthesized = synthesizeCodeFromHunks(diffLines: diffLines, hunks: hunks)
        onApplyCode(synthesized)
    }
}
