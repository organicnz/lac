import Foundation

// MARK: - Chat persistence + gateway client (local or Tailscale remote via LACConnectionStore)

public struct MessageVariant: Codable, Identifiable, Hashable, Sendable {
    public var id: UUID
    public var content: String
    public var model: String?
    public var durationSeconds: Double?
    public var tokensPerSecond: Double?
    public var ts: UInt64

    public init(
        id: UUID = UUID(),
        content: String,
        model: String? = nil,
        durationSeconds: Double? = nil,
        tokensPerSecond: Double? = nil,
        ts: UInt64 = UInt64(Date().timeIntervalSince1970)
    ) {
        self.id = id
        self.content = content
        self.model = model
        self.durationSeconds = durationSeconds
        self.tokensPerSecond = tokensPerSecond
        self.ts = ts
    }
}

struct ChatMessage: Codable, Identifiable {
    var id: UUID
    var role: String // "user" | "assistant"
    var content: String
    var ts: UInt64
    var model: String?
    var durationSeconds: Double?
    var tokensPerSecond: Double?
    var variants: [MessageVariant]
    var selectedVariantIndex: Int

    init(
        id: UUID = UUID(),
        role: String,
        content: String,
        ts: UInt64 = UInt64(Date().timeIntervalSince1970),
        model: String? = nil,
        durationSeconds: Double? = nil,
        tokensPerSecond: Double? = nil,
        variants: [MessageVariant] = [],
        selectedVariantIndex: Int = 0
    ) {
        self.id = id
        self.role = role
        self.content = content
        self.ts = ts
        self.model = model
        self.durationSeconds = durationSeconds
        self.tokensPerSecond = tokensPerSecond
        self.variants = variants
        self.selectedVariantIndex = selectedVariantIndex
    }
}

private struct StoredLine: Codable {
    var thread: String
    var id: UUID?
    var role: String
    var content: String
    var ts: UInt64
    var model: String?
    var durationSeconds: Double?
    var tokensPerSecond: Double?
    var variants: [MessageVariant]?
    var selectedVariantIndex: Int?
}

struct ChatThread: Identifiable {
    var id: String
    var title: String
    var messages: [ChatMessage]

    var relativeTime: String {
        guard let last = messages.last else { return "New" }
        let date = Date(timeIntervalSince1970: TimeInterval(last.ts))
        let formatter = RelativeDateTimeFormatter()
        formatter.unitsStyle = .abbreviated
        return formatter.localizedString(for: date, relativeTo: Date())
    }
}

private struct ModelsResponse: Codable {
    var data: [ModelEntry]?
}

private struct ModelEntry: Codable {
    var id: String?
}

private struct StreamChunk: Codable {
    var choices: [StreamChoice]?
}

private struct StreamChoice: Codable {
    var delta: StreamDelta?
}

private struct StreamDelta: Codable {
    var content: String?
    var reasoning_content: String?
}


enum ChatError: LocalizedError {
    case http(Int, String)
    case transport(String)

    var errorDescription: String? {
        switch self {
        case .http(let code, let detail):
            if code == 502 || code == 503 {
                return "No backend served the request (HTTP \(code)). Start one with `lac serve mlx`.\(detail.isEmpty ? "" : " \(detail)")"
            }
            return "Gateway HTTP \(code).\(detail.isEmpty ? "" : " \(detail)")"
        case .transport(let what):
            return "Request failed: \(what)"
        }
    }
}

@MainActor
class ChatStore: ObservableObject {
    @Published var threads: [ChatThread] = []
    @Published var activeThreadId: String?
    @Published var models: [String] = []
    @Published var selectedModel: String = ProcessInfo.processInfo.environment["LAC_CHAT_MODEL"] ?? ""
    @Published var isSending = false
    @Published var streamText = ""
    @Published var errorText: String?

    // Hyperparameters & System Tuning (AGENTS.md standards)
    @Published var temperature: Double = 0.6
    @Published var topP: Double = 0.95
    @Published var contextCap: Int = 32768
    @Published var thinkingMode: Bool = false
    @Published var customSystemPrompt: String = "You are LAC Assistant, a world-class local agentic AI running on Apple Silicon. You are concise, precise, memory-efficient, and generate clean, robust solutions."

    var connection: LACConnectionStore { LACConnectionStore.shared }
    public var port: Int { connection.port }

    private var streamTask: Task<Void, Never>?

    var activeThread: ChatThread? {
        threads.first { $0.id == activeThreadId }
    }

    static func threadsFile() -> URL {
        URL(fileURLWithPath: NSHomeDirectory()).appendingPathComponent(".lac/chat-threads.jsonl")
    }

    static func titlesFile() -> URL {
        URL(fileURLWithPath: NSHomeDirectory()).appendingPathComponent(".lac/chat-titles.json")
    }

    /// Local 27B KV budget: never request more than 4k completion tokens —
    /// contextCap/2 at 64k would OOM unified memory mid-stream.
    nonisolated static func maxTokens(forContextCap cap: Int) -> Int? {
        guard cap > 0 else { return nil }
        return min(cap / 2, 4096)
    }

    init() {
        load()
        if activeThreadId == nil { _ = newThread() }
    }

    // MARK: threads

    @discardableResult
    func newThread() -> ChatThread {
        let t = ChatThread(id: UUID().uuidString, title: "New chat", messages: [])
        threads.insert(t, at: 0)
        activeThreadId = t.id
        saveTitles()
        return t
    }

    func select(_ id: String) {
        // Select first, then prune *other* empty threads (never the selected one).
        // Deleting the selected id leaves a dangling activeThreadId.
        threads.removeAll(where: { $0.id != id && $0.messages.isEmpty })
        guard threads.contains(where: { $0.id == id }) else {
            // Selected thread no longer exists — fall back to most recent.
            activeThreadId = threads.first?.id
            if activeThreadId == nil { _ = newThread() }
            streamText = ""
            errorText = nil
            return
        }
        activeThreadId = id
        streamText = ""
        errorText = nil
    }

    func deleteThread(_ id: String) {
        if let idx = threads.firstIndex(where: { $0.id == id }) {
            threads.remove(at: idx)
            rewriteAllThreads()
            if activeThreadId == id {
                activeThreadId = threads.first?.id
                if activeThreadId == nil {
                    _ = newThread()
                }
            }
        }
    }

    func renameThread(_ id: String, title: String) {
        let trimmed = title.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        if let idx = threads.firstIndex(where: { $0.id == id }) {
            threads[idx].title = trimmed
            rewriteAllThreads()
        }
    }

    func clearActiveThread() {
        guard let tid = activeThreadId,
              let idx = threads.firstIndex(where: { $0.id == tid }) else { return }
        
        // Remove messages and empty thread
        threads[idx].messages.removeAll()
        
        // Remove empty threads (like frontier LLM companies do)
        if threads[idx].messages.isEmpty {
            threads.remove(at: idx)
            // Re-point selection when the active thread was removed;
            // otherwise activeThread resolves to nil and the UI blanks.
            if activeThreadId == tid {
                activeThreadId = threads.first?.id
            }
        }
        
        rewriteAllThreads()
        errorText = nil
        streamText = ""
        
        // If no threads left, create a new one
        if threads.isEmpty {
            _ = newThread()
        }
    }

    func deleteMessage(id: UUID) {
        guard let tid = activeThreadId,
              let idx = threads.firstIndex(where: { $0.id == tid }) else { return }
        threads[idx].messages.removeAll { $0.id == id }
        rewriteAllThreads()
    }

    @discardableResult
    func forkThread(at messageId: UUID) -> ChatThread? {
        guard let tid = activeThreadId,
              let currentThread = threads.first(where: { $0.id == tid }),
              let msgIdx = currentThread.messages.firstIndex(where: { $0.id == messageId }) else { return nil }

        let forkedMessages = Array(currentThread.messages[...msgIdx])
        let newTid = UUID().uuidString
        let newTitle = "\(currentThread.title) (Fork)"
        let newThread = ChatThread(id: newTid, title: newTitle, messages: forkedMessages)
        threads.insert(newThread, at: 0)
        activeThreadId = newTid
        rewriteAllThreads()
        LiquidGlass.haptic(.alignment)
        return newThread
    }

    func regenerateLastResponse(withModel: String? = nil) {
        guard !isSending else { return }
        guard let tid = activeThreadId,
              let idx = threads.firstIndex(where: { $0.id == tid }) else { return }
        
        var replaceId: UUID? = nil
        if let last = threads[idx].messages.last, last.role == "assistant" {
            replaceId = last.id
        }
        guard !threads[idx].messages.isEmpty else { return }
        if let m = withModel, !m.isEmpty {
            selectedModel = m
        }
        errorText = nil
        let model = selectedModel.isEmpty ? (models.first ?? "mlx-community/Qwen3.8-27B-4bit") : selectedModel
        let history: [ChatMessage]
        if replaceId != nil {
            history = Array(threads[idx].messages.dropLast())
        } else {
            history = threads[idx].messages
        }
        executeStream(history: history, tid: tid, model: model, replaceAssistantMessageId: replaceId)
    }

    func selectVariant(messageId: UUID, index: Int) {
        guard let tid = activeThreadId,
              let tIdx = threads.firstIndex(where: { $0.id == tid }),
              let mIdx = threads[tIdx].messages.firstIndex(where: { $0.id == messageId }) else { return }
        var msg = threads[tIdx].messages[mIdx]
        guard index >= 0 && index < msg.variants.count else { return }
        msg.selectedVariantIndex = index
        let v = msg.variants[index]
        msg.content = v.content
        msg.model = v.model
        msg.durationSeconds = v.durationSeconds
        msg.tokensPerSecond = v.tokensPerSecond
        threads[tIdx].messages[mIdx] = msg
        rewriteAllThreads()
        LiquidGlass.haptic(.alignment)
    }

    func editAndResend(userMessageId: UUID, newPrompt: String) {
        guard !isSending else { return }
        let text = newPrompt.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return }
        guard let tid = activeThreadId,
              let idx = threads.firstIndex(where: { $0.id == tid }) else { return }
        guard let msgIdx = threads[idx].messages.firstIndex(where: { $0.id == userMessageId }) else { return }
        threads[idx].messages.removeSubrange(msgIdx...)
        let updatedUser = ChatMessage(role: "user", content: text)
        threads[idx].messages.append(updatedUser)
        rewriteAllThreads()
        errorText = nil
        let model = selectedModel.isEmpty ? (models.first ?? "mlx-community/Qwen3.8-27B-4bit") : selectedModel
        let history = threads[idx].messages
        executeStream(history: history, tid: tid, model: model)
    }

    func retryLastPrompt() {
        guard let tid = activeThreadId,
              let thread = threads.first(where: { $0.id == tid }),
              thread.messages.contains(where: { $0.role == "user" }) else { return }
        errorText = nil
        regenerateLastResponse()
    }

    func friendlyName(for modelId: String) -> String {
        let raw = modelId.trimmingCharacters(in: .whitespacesAndNewlines)
        if raw.isEmpty { return "Qwen 3.8 27B · Q4 MLX" }
        if raw.contains("Qwen3.8-27B") || raw.contains("qwen3.8-27b") {
            if raw.contains("8bit") || raw.contains("8-bit") || raw.contains("Q8") {
                return "Qwen 3.8 27B · Q8 llama"
            }
            return "Qwen 3.8 27B · Q4 MLX"
        }
        if raw.contains("Qwen2.5-Coder-32B") {
            return "Qwen 2.5 Coder 32B"
        }
        if raw.contains("DeepSeek") {
            return "DeepSeek Coder"
        }
        if let last = raw.split(separator: "/").last {
            return String(last)
        }
        return raw
    }

    var friendlyModelName: String {
        friendlyName(for: selectedModel)
    }

    private func rewriteAllThreads() {
        let file = Self.threadsFile()
        try? FileManager.default.createDirectory(
            at: file.deletingLastPathComponent(), withIntermediateDirectories: true)
        var buffer = Data()
        let enc = JSONEncoder()
        for thread in threads {
            for msg in thread.messages {
                if let line = try? enc.encode(StoredLine(
                    thread: thread.id, id: msg.id, role: msg.role, content: msg.content, ts: msg.ts, model: msg.model,
                    durationSeconds: msg.durationSeconds, tokensPerSecond: msg.tokensPerSecond,
                    variants: msg.variants.isEmpty ? nil : msg.variants,
                    selectedVariantIndex: msg.selectedVariantIndex
                )) {
                    buffer.append(line)
                    buffer.append(0x0A)
                }
            }
        }
        try? buffer.write(to: file, options: .atomic)
        // Titles live in a sidecar (the JSONL carries messages only),
        // so renames and forks survive restarts.
        saveTitles()
    }

    private func saveTitles() {
        let file = Self.titlesFile()
        try? FileManager.default.createDirectory(
            at: file.deletingLastPathComponent(), withIntermediateDirectories: true)
        let map = Dictionary(uniqueKeysWithValues: threads.map { ($0.id, $0.title) })
        if let data = try? JSONEncoder().encode(map) {
            try? data.write(to: file, options: .atomic)
        }
    }

    /// Derive a meaningful thread title from message content
    private func deriveThreadTitle(from messages: [ChatMessage]) -> String {
        // Strategy 1: Look for a concise user prompt (preferred)
        if let firstUser = messages.first(where: { $0.role == "user" && !$0.content.isEmpty }) {
            let truncated = firstUser.content.prefix(45).trimmingCharacters(in: .whitespacesAndNewlines)
            if !truncated.isEmpty && truncated.count > 3 {
                return truncated + (firstUser.content.count > 45 ? "..." : "")
            }
        }
        
        // Strategy 2: Use the most recent substantial message (any role)
        if let lastMsg = messages.last, !lastMsg.content.isEmpty {
            let truncated = lastMsg.content.prefix(40).trimmingCharacters(in: .whitespacesAndNewlines)
            if !truncated.isEmpty {
                return truncated + (lastMsg.content.count > 40 ? "..." : "")
            }
        }
        
        // Strategy 3: Shorten to first 20 chars of any content
        if !messages.isEmpty, let firstMsg = messages.first, !firstMsg.content.isEmpty {
            let truncated = firstMsg.content.prefix(20).trimmingCharacters(in: .whitespacesAndNewlines)
            return truncated + (firstMsg.content.count > 20 ? "..." : "")
        }
        
        return "Chat"
    }

    private func loadTitles() -> [String: String] {
        guard let data = try? Data(contentsOf: Self.titlesFile()),
              let map = try? JSONDecoder().decode([String: String].self, from: data)
        else { return [:] }
        return map
    }

    // MARK: persistence (one JSONL line per message)

    func load() {
        let file = Self.threadsFile()
        let exists = FileManager.default.fileExists(atPath: file.path)
        
        // Robustness: gracefully handle missing or corrupted file
        guard exists,
              let data = try? Data(contentsOf: file),
              let text = String(data: data, encoding: .utf8) else {
            // No file or corrupted - start fresh with new thread
            threads = []
            activeThreadId = nil
            _ = newThread()
            return
        }
        
        // Quick exit for empty files
        if text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            threads = []
            activeThreadId = nil
            _ = newThread()
            return
        }
        
        var grouped: [String: [ChatMessage]] = [:]
        var order: [String] = []
        let dec = JSONDecoder()
        
        for line in text.split(separator: "\n") {
            // Skip truly empty lines
            guard !line.isEmpty,
                  let d = line.data(using: .utf8),
                  let s = try? dec.decode(StoredLine.self, from: d) else { continue }
            
            // Track thread appearance order (first-seen recency)
            if grouped[s.thread] == nil { order.append(s.thread) }
            
            let loadedVariants = s.variants ?? (s.role == "assistant" ? [
                MessageVariant(id: s.id ?? UUID(), content: s.content, model: s.model, durationSeconds: s.durationSeconds, tokensPerSecond: s.tokensPerSecond, ts: s.ts)
            ] : [])

            grouped[s.thread, default: []].append(
                ChatMessage(
                    id: s.id ?? UUID(),
                    role: s.role,
                    content: s.content,
                    ts: s.ts,
                    model: s.model,
                    durationSeconds: s.durationSeconds,
                    tokensPerSecond: s.tokensPerSecond,
                    variants: loadedVariants,
                    selectedVariantIndex: s.selectedVariantIndex ?? 0
                )
            )
        }
        
        // Messages ascending (oldest→newest) for transcript + title derivation;
        // threads newest-activity-first for sidebar.
        let saved = loadTitles()
        var built = order.compactMap { tid -> ChatThread? in
            let msgs = (grouped[tid] ?? []).sorted { $0.ts < $1.ts }
            guard !msgs.isEmpty else { return nil }
            let title = saved[tid] ?? self.deriveThreadTitle(from: msgs)
            return ChatThread(id: tid, title: title, messages: msgs)
        }
        built.sort { ($0.messages.last?.ts ?? 0) > ($1.messages.last?.ts ?? 0) }
        threads = built
        
        // If no valid threads remain, start fresh
        if threads.isEmpty {
            threads = []
            activeThreadId = nil
            _ = newThread()
            return
        }
        
        // Intelligence: set active thread to most recently active,
        // but preserve user-selected thread if it still exists
        if activeThreadId == nil || !threads.contains(where: { $0.id == activeThreadId }) {
            activeThreadId = threads.first?.id  // Most recent
        }
    }

    private func persist(_ threadId: String, _ msg: ChatMessage) {
        let file = Self.threadsFile()
        try? FileManager.default.createDirectory(
            at: file.deletingLastPathComponent(), withIntermediateDirectories: true)
        guard let payload = try? JSONEncoder().encode(
            StoredLine(
                thread: threadId,
                id: msg.id,
                role: msg.role,
                content: msg.content,
                ts: msg.ts,
                model: msg.model,
                durationSeconds: msg.durationSeconds,
                tokensPerSecond: msg.tokensPerSecond,
                variants: msg.variants.isEmpty ? nil : msg.variants,
                selectedVariantIndex: msg.selectedVariantIndex
            )
        ) else { return }
        if let handle = try? FileHandle(forWritingTo: file) {
            defer { _ = try? handle.close() }
            _ = try? handle.seekToEnd()
            var withNL = payload
            withNL.append(0x0A)
            try? handle.write(contentsOf: withNL)
        } else {
            // First write: create the file atomically.
            var withNL = payload
            withNL.append(0x0A)
            try? withNL.write(to: file, options: .atomic)
        }
    }

    // MARK: models

    func fetchModels() async {
        guard let url = connection.url(path: "/v1/models") else { return }
        var req = URLRequest(url: url)
        req.timeoutInterval = 8
        connection.authorize(&req)
        do {
            let (data, _) = try await URLSession.shared.data(for: req)
            let decoded = try JSONDecoder().decode(ModelsResponse.self, from: data)
            let ids = (decoded.data ?? []).compactMap(\.id).filter { !$0.isEmpty }
            models = ids
            if selectedModel.isEmpty, let first = ids.first {
                selectedModel = first
            } else if selectedModel.isEmpty {
                selectedModel = "mlx-community/Qwen3.8-27B-4bit"
            }
        } catch {
            models = []
            if selectedModel.isEmpty {
                selectedModel = "mlx-community/Qwen3.8-27B-4bit"
            }
        }
    }

    // MARK: send (streaming SSE)

    func send(_ prompt: String) {
        let text = prompt.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty, !isSending else { return }
        if activeThreadId == nil || !threads.contains(where: { $0.id == activeThreadId }) {
            if let first = threads.first(where: { !$0.messages.isEmpty }) ?? threads.first {
                activeThreadId = first.id
            } else {
                activeThreadId = newThread().id
            }
        }
        guard let tid = activeThreadId,
              let idx = threads.firstIndex(where: { $0.id == tid }) else { return }
        errorText = nil
        if selectedModel.isEmpty {
            selectedModel = models.first ?? "mlx-community/Qwen3.8-27B-4bit"
        }
        let model = selectedModel.trimmingCharacters(in: .whitespacesAndNewlines)
        let user = ChatMessage(role: "user", content: text)
        threads[idx].messages.append(user)
        persist(tid, user)
        if threads[idx].messages.count == 1 {
            threads[idx].title = String(text.prefix(40))
        }
        let history = threads[idx].messages
        executeStream(history: history, tid: tid, model: model)
    }

    func executeStream(history: [ChatMessage], tid: String, model: String, replaceAssistantMessageId: UUID? = nil) {
        guard !isSending else { return }
        isSending = true
        streamText = ""
        let tStart = CFAbsoluteTimeGetCurrent()
        streamTask = Task { @MainActor [weak self, tid] in
            guard let self else { return }
            defer {
                self.isSending = false
            }
            do {
                // onPiece runs on MainActor already (streamChat is isolated),
                // so the transcript updates inline with no hop.
                let reply = try await self.streamChat(model: model, history: history) { piece in
                    self.streamText += piece
                }
                let duration = CFAbsoluteTimeGetCurrent() - tStart
                let estTokens = max(1, reply.count / 4)
                let tokPerSec = duration > 0.05 ? Double(estTokens) / duration : 0.0

                guard !Task.isCancelled else {
                    if !self.streamText.isEmpty {
                        let partial = ChatMessage(
                            role: "assistant",
                            content: self.streamText + " [Interrupted]",
                            model: model,
                            durationSeconds: duration,
                            tokensPerSecond: tokPerSec
                        )
                        if let live = self.threads.firstIndex(where: { $0.id == tid }) {
                            self.threads[live].messages.append(partial)
                        }
                        self.persist(tid, partial)
                        self.streamText = ""
                    }
                    return
                }

                if let live = self.threads.firstIndex(where: { $0.id == tid }) {
                    if let rId = replaceAssistantMessageId,
                       let mIdx = self.threads[live].messages.firstIndex(where: { $0.id == rId }) {
                        var existing = self.threads[live].messages[mIdx]
                        if existing.variants.isEmpty {
                            existing.variants.append(MessageVariant(
                                id: existing.id,
                                content: existing.content,
                                model: existing.model,
                                durationSeconds: existing.durationSeconds,
                                tokensPerSecond: existing.tokensPerSecond,
                                ts: existing.ts
                            ))
                        }
                        let newVariant = MessageVariant(
                            id: UUID(),
                            content: reply,
                            model: model,
                            durationSeconds: duration,
                            tokensPerSecond: tokPerSec,
                            ts: UInt64(Date().timeIntervalSince1970)
                        )
                        existing.variants.append(newVariant)
                        existing.selectedVariantIndex = existing.variants.count - 1
                        existing.content = reply
                        existing.model = model
                        existing.durationSeconds = duration
                        existing.tokensPerSecond = tokPerSec
                        self.threads[live].messages[mIdx] = existing
                        self.rewriteAllThreads()
                    } else {
                        var assistant = ChatMessage(
                            role: "assistant",
                            content: reply,
                            model: model,
                            durationSeconds: duration,
                            tokensPerSecond: tokPerSec
                        )
                        assistant.variants = [MessageVariant(
                            id: assistant.id,
                            content: reply,
                            model: model,
                            durationSeconds: duration,
                            tokensPerSecond: tokPerSec,
                            ts: assistant.ts
                        )]
                        assistant.selectedVariantIndex = 0
                        self.threads[live].messages.append(assistant)
                        self.persist(tid, assistant)
                    }
                }
                self.streamText = ""
            } catch is CancellationError {
                let duration = CFAbsoluteTimeGetCurrent() - tStart
                if !self.streamText.isEmpty {
                    let partial = ChatMessage(
                        role: "assistant",
                        content: self.streamText + " [Interrupted]",
                        model: model,
                        durationSeconds: duration
                    )
                    if let live = self.threads.firstIndex(where: { $0.id == tid }) {
                        self.threads[live].messages.append(partial)
                    }
                    self.persist(tid, partial)
                    self.streamText = ""
                }
            } catch {
                let duration = CFAbsoluteTimeGetCurrent() - tStart
                if !self.streamText.isEmpty {
                    let partial = ChatMessage(
                        role: "assistant",
                        content: self.streamText,
                        model: model,
                        durationSeconds: duration
                    )
                    if let live = self.threads.firstIndex(where: { $0.id == tid }) {
                        self.threads[live].messages.append(partial)
                    }
                    self.persist(tid, partial)
                    self.streamText = ""
                }
                self.errorText = (error as? LocalizedError)?.errorDescription ?? error.localizedDescription
            }
        }
    }

    func cancel() {
        streamTask?.cancel()
        streamTask = nil
        isSending = false
    }

    private func streamChat(
        model: String,
        history: [ChatMessage],
        onPiece: @MainActor @Sendable @escaping (String) -> Void
    ) async throws -> String {
        guard let url = connection.url(path: "/v1/chat/completions") else {
            throw ChatError.transport("bad gateway URL")
        }
        struct WireMessage: Encodable { var role: String; var content: String }
        struct WireRequest: Encodable {
            var model: String
            var messages: [WireMessage]
            var stream: Bool
            var temperature: Double?
            var top_p: Double?
            var max_tokens: Int?
        }

        var wireMessages: [WireMessage] = []
        if !history.contains(where: { $0.role == "system" }) {
            var sys = self.customSystemPrompt
            if self.thinkingMode {
                sys += "\nThinking mode enabled: Provide deep reasoning within <think> tags before delivering output."
            }
            wireMessages.append(WireMessage(role: "system", content: sys))
        }
        wireMessages.append(contentsOf: history.map { WireMessage(role: $0.role, content: $0.content) })

        var req = URLRequest(url: url)
        req.httpMethod = "POST"
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        connection.authorize(&req)
        req.timeoutInterval = 300
        req.httpBody = try JSONEncoder().encode(WireRequest(
            model: model,
            messages: wireMessages,
            stream: true,
            temperature: self.temperature,
            top_p: self.topP,
            max_tokens: Self.maxTokens(forContextCap: self.contextCap)
        ))
        let (bytes, response): (URLSession.AsyncBytes, URLResponse)
        do {
            (bytes, response) = try await URLSession.shared.bytes(for: req)
        } catch {
            throw ChatError.transport(error.localizedDescription)
        }
        guard let http = response as? HTTPURLResponse else {
            throw ChatError.transport("non-HTTP response")
        }
        guard http.statusCode == 200 else {
            // Drain a short error body for the message, handling both JSON and SSE.
            var detail = ""
            do {
                var count = 0
                for try await line in bytes.lines {
                    let t = line.trimmingCharacters(in: .whitespaces)
                    if t.hasPrefix("data:") {
                        let payload = String(t.dropFirst(5)).trimmingCharacters(in: .whitespaces)
                        if let d = payload.data(using: .utf8),
                           let e = try? JSONDecoder().decode(StreamError.self, from: d),
                           let m = e.error?.message, !m.isEmpty {
                            detail = String(m.prefix(300))
                            break
                        }
                    } else if let d = t.data(using: .utf8),
                              let e = try? JSONDecoder().decode(StreamError.self, from: d),
                              let m = e.error?.message, !m.isEmpty {
                        detail = String(m.prefix(300))
                        break
                    } else if !t.isEmpty && detail.isEmpty {
                        detail = String(t.prefix(300))
                    }
                    count += 1
                    if count > 20 { break }
                }
            } catch { /* keep generic */ }
            throw ChatError.http(http.statusCode, detail)
        }
        var full = ""
        var isThinking = false
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
                if let reason = choice.delta?.reasoning_content, !reason.isEmpty {
                    if !isThinking {
                        isThinking = true
                        full += "<think>\n"
                        onPiece("<think>\n")
                    }
                    full += reason
                    onPiece(reason)
                }
                if let piece = choice.delta?.content, !piece.isEmpty {
                    if isThinking {
                        isThinking = false
                        full += "\n</think>\n"
                        onPiece("\n</think>\n")
                    }
                    full += piece
                    onPiece(piece)
                }
            }
        }
        if isThinking {
            full += "\n</think>\n"
            onPiece("\n</think>\n")
        }
        return full
    }
}

private struct StreamError: Codable {
    var error: StreamErrorBody?
}

private struct StreamErrorBody: Codable {
    var message: String?
}
