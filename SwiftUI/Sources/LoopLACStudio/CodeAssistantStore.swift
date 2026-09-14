import Foundation
import SwiftUI

// MARK: - Code Assistant Actions

public enum CodeAssistantAction: String, CaseIterable, Identifiable {
    case explain = "Explain Code"
    case refactor = "Refactor / Clean"
    case tests = "Generate Tests"
    case audit = "Audit Bugs & Safety"
    case optimize = "Optimize (SIMD / Cache)"

    public var id: String { rawValue }

    public var icon: String {
        switch self {
        case .explain: return "text.magnifyingglass"
        case .refactor: return "wand.and.stars"
        case .tests: return "checkmark.seal.fill"
        case .audit: return "shield.lefthalf.filled"
        case .optimize: return "speedometer"
        }
    }

    public var systemInstruction: String {
        switch self {
        case .explain:
            return "Explain this code in depth: architectural invariants, data flow, memory model, and key algorithmic properties."
        case .refactor:
            return "Refactor this code to be maximally idiomatic, readable, memory-safe, and robust. Provide the complete refactored version in a fenced code block."
        case .tests:
            return "Generate comprehensive, production-grade unit tests for this code. Cover normal cases, edge cases, error conditions, and bounds checks."
        case .audit:
            return "Audit this code for bugs, race conditions, memory leaks, panic vulnerabilities, and concurrency hazards. Highlight each issue and how to fix it."
        case .optimize:
            return "Analyze this code for performance bottlenecks, cache locality, unnecessary allocations, and Apple Silicon SIMD / vectorization opportunities."
        }
    }
}

// MARK: - Working Set Editor Tab

public struct EditorTab: Identifiable, Equatable, Sendable {
    public let id: UUID
    public var url: URL?
    public var title: String
    public var content: String
    public var language: String
    public var isModified: Bool

    public init(
        id: UUID = UUID(),
        url: URL? = nil,
        title: String,
        content: String,
        language: String = "Rust",
        isModified: Bool = false
    ) {
        self.id = id
        self.url = url
        self.title = title
        self.content = content
        self.language = language
        self.isModified = isModified
    }
}

// MARK: - Console Command Request (Command Palette ⌘K → Agent Console)

public enum ConsoleCommand: String, Sendable {
    case cargoTests = "cargoTests"
    case swiftTests = "swiftTests"
    case gitDiff = "gitDiff"
}

// MARK: - Code Assistant Store

@MainActor
public class CodeAssistantStore: ObservableObject {
    @Published public var tabs: [EditorTab] = []
    @Published public var activeTabId: UUID = UUID()

    @Published public var sourceCode: String = "" {
        didSet {
            // Keep active tab synchronized with editor changes
            if let idx = tabs.firstIndex(where: { $0.id == activeTabId }) {
                if tabs[idx].content != sourceCode {
                    tabs[idx].content = sourceCode
                    tabs[idx].isModified = true
                }
            }
        }
    }
    @Published public var selectedLanguage: String = "Rust" {
        didSet {
            if let idx = tabs.firstIndex(where: { $0.id == activeTabId }) {
                tabs[idx].language = selectedLanguage
            }
        }
    }
    @Published public var customPrompt: String = ""
    @Published public var isProcessing: Bool = false
    @Published public var streamResponse: String = ""
    @Published public var lastAppliedCode: String?
    @Published public var errorText: String?
    /// Set by the ⌘K palette; CodeAssistantView consumes it on arrival
    /// (runs the console command, reveals the drawer, clears to nil).
    @Published public var pendingConsoleCommand: ConsoleCommand?

    public static let availableLanguages = [
        "Rust", "Swift", "Python", "TypeScript", "Go", "C++", "C", "Shell", "SQL", "HTML/CSS"
    ]

    public static let defaultSnippet = """
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

    private let port: Int = {
        if let raw = ProcessInfo.processInfo.environment["LAC_ROUTER_PORT"],
           let p = Int(raw), p > 0 { return p }
        return 8000
    }()

    private var streamTask: Task<Void, Never>?

    public init() {
        let initialTab = EditorTab(
            title: "RingBuffer.rs",
            content: Self.defaultSnippet,
            language: "Rust",
            isModified: false
        )
        self.tabs = [initialTab]
        self.activeTabId = initialTab.id
        self.sourceCode = initialTab.content
        self.selectedLanguage = initialTab.language
    }

    // MARK: Working Set Tab Management

    public var activeTab: EditorTab? {
        tabs.first(where: { $0.id == activeTabId })
    }

    public nonisolated static func language(for url: URL) -> String {
        let ext = url.pathExtension.lowercased()
        switch ext {
        case "rs": return "Rust"
        case "swift": return "Swift"
        case "py": return "Python"
        case "ts", "tsx", "js", "jsx": return "TypeScript"
        case "go": return "Go"
        case "cpp", "cc", "cxx", "hpp", "h": return "C++"
        case "c": return "C"
        case "sh", "bash", "zsh": return "Shell"
        case "sql": return "SQL"
        case "html", "css": return "HTML/CSS"
        case "json", "toml", "yaml", "yml": return "Rust"
        default: return "Rust"
        }
    }

    public func openFile(url: URL, content: String) {
        if let existing = tabs.first(where: { $0.url?.path == url.path }) {
            selectTab(id: existing.id)
            return
        }

        let lang = Self.language(for: url)
        let tab = EditorTab(
            url: url,
            title: url.lastPathComponent,
            content: content,
            language: lang,
            isModified: false
        )
        tabs.append(tab)
        selectTab(id: tab.id)
    }

    public func newTab() {
        let tab = EditorTab(
            title: "Untitled-\(tabs.count + 1)",
            content: "",
            language: selectedLanguage,
            isModified: false
        )
        tabs.append(tab)
        selectTab(id: tab.id)
    }

    public func selectTab(id: UUID) {
        guard let tab = tabs.first(where: { $0.id == id }) else { return }
        let currentIsModified = tab.isModified
        activeTabId = id
        sourceCode = tab.content
        selectedLanguage = tab.language
        if let idx = tabs.firstIndex(where: { $0.id == id }) {
            tabs[idx].isModified = currentIsModified
        }
    }

    public func closeTab(id: UUID) {
        guard let idx = tabs.firstIndex(where: { $0.id == id }) else { return }
        let closingActive = (id == activeTabId)
        tabs.remove(at: idx)

        if tabs.isEmpty {
            newTab()
        } else if closingActive {
            let nextIndex = min(idx, tabs.count - 1)
            selectTab(id: tabs[nextIndex].id)
        }
    }

    public func closeOtherTabs(id: UUID) {
        tabs.removeAll { $0.id != id }
        selectTab(id: id)
    }

    public func closeAllTabs() {
        tabs.removeAll()
        newTab()
    }

    public func saveAllTabs() {
        for i in tabs.indices {
            if let url = tabs[i].url, tabs[i].isModified {
                try? tabs[i].content.write(to: url, atomically: true, encoding: .utf8)
                tabs[i].isModified = false
            }
        }
        LiquidGlass.haptic(.alignment)
    }

    public func saveActiveFile() {
        guard let idx = tabs.firstIndex(where: { $0.id == activeTabId }),
              let url = tabs[idx].url else { return }
        do {
            try sourceCode.write(to: url, atomically: true, encoding: .utf8)
            tabs[idx].content = sourceCode
            tabs[idx].isModified = false
            LiquidGlass.haptic(.alignment)
        } catch {
            self.errorText = "Failed to save \(url.lastPathComponent): \(error.localizedDescription)"
        }
    }

    // MARK: Execution

    public func executeAction(_ action: CodeAssistantAction) {
        let taskText = action.systemInstruction
        runTask(instruction: taskText)
    }

    public func executeCustomPrompt() {
        let prompt = customPrompt.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !prompt.isEmpty else { return }
        runTask(instruction: prompt)
    }

    private func runTask(instruction: String) {
        guard !isProcessing else { return }
        errorText = nil
        streamResponse = ""
        isProcessing = true

        let code = sourceCode.trimmingCharacters(in: .whitespacesAndNewlines)
        let lang = selectedLanguage

        let fullPrompt: String
        if code.isEmpty {
            fullPrompt = instruction
        } else {
            fullPrompt = """
Language: \(lang)

```\(lang.lowercased())
\(code)
```

Task: \(instruction)
"""
        }

        streamTask = Task { [weak self] in
            guard let self else { return }
            defer { self.isProcessing = false }

            do {
                let full = try await self.streamCompletion(prompt: fullPrompt)
                guard !Task.isCancelled else { return }
                self.streamResponse = full
            } catch is CancellationError {
                if !self.streamResponse.isEmpty {
                    self.streamResponse += "\n\n*[Task Interrupted]*"
                }
            } catch {
                self.errorText = error.localizedDescription
            }
        }
    }

    public func cancel() {
        streamTask?.cancel()
        streamTask = nil
        isProcessing = false
    }

    public func applyToEditor(_ codeBlock: String) {
        lastAppliedCode = sourceCode
        sourceCode = codeBlock
        LiquidGlass.haptic(.alignment)
    }

    public func undoApply() {
        if let prev = lastAppliedCode {
            sourceCode = prev
            lastAppliedCode = nil
            LiquidGlass.haptic(.alignment)
        }
    }

    // MARK: Streaming Network Client

    private func streamCompletion(prompt: String) async throws -> String {
        guard let url = URL(string: "http://127.0.0.1:\(port)/v1/chat/completions") else {
            throw URLError(.badURL)
        }

        let systemMsg = "You are LAC Code Assistant, a world-class systems and software engineering agent running locally on Apple Silicon. You write clean, idiomatic, robust, memory-safe code with zero unnecessary dependencies. Provide exact code, explanations, and diffs where applicable."

        struct WireMessage: Encodable { var role: String; var content: String }
        struct WireRequest: Encodable {
            var model: String
            var messages: [WireMessage]
            var stream: Bool
            var temperature: Double
        }

        let model = ProcessInfo.processInfo.environment["LAC_CODE_MODEL"] ??
                    ProcessInfo.processInfo.environment["LAC_CHAT_MODEL"] ??
                    "mlx-community/Qwen3.8-27B-4bit"

        var req = URLRequest(url: url)
        req.httpMethod = "POST"
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        req.timeoutInterval = 300
        req.httpBody = try JSONEncoder().encode(WireRequest(
            model: model,
            messages: [
                WireMessage(role: "system", content: systemMsg),
                WireMessage(role: "user", content: prompt)
            ],
            stream: true,
            temperature: 0.2
        ))

        let (bytes, response) = try await URLSession.shared.bytes(for: req)
        guard let http = response as? HTTPURLResponse else {
            throw URLError(.badServerResponse)
        }
        guard http.statusCode == 200 else {
            throw URLError(.init(rawValue: http.statusCode))
        }

        var full = ""
        let dec = JSONDecoder()

        for try await line in bytes.lines {
            try Task.checkCancellation()
            let t = line.trimmingCharacters(in: .whitespaces)
            guard t.hasPrefix("data:") else { continue }
            let payload = String(t.dropFirst(5)).trimmingCharacters(in: .whitespaces)
            if payload == "[DONE]" { break }

            guard let d = payload.data(using: .utf8),
                  let chunk = try? dec.decode(StreamChunk.self, from: d) else { continue }

            for choice in chunk.choices ?? [] {
                if let piece = choice.delta?.content, !piece.isEmpty {
                    full += piece
                    self.streamResponse = full
                }
            }
        }
        return full
    }
}

private struct StreamChunk: Codable {
    var choices: [StreamChoice]?
}

private struct StreamChoice: Codable {
    var delta: StreamDelta?
}

private struct StreamDelta: Codable {
    var content: String?
}
