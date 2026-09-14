import Foundation
import Testing

@testable import LoopLACStudio

// The live gateway may be v2.0 (old daemon) or v2.7+: the dashboard
// must decode both instead of blanking on a missing key.
struct RouterDecodeTests {
    // Exact v2.0 shape observed on :8000 (no uptime/inflight/stats).
    static let v20 = """
    {"status":"ok","router":"lac-router v2.0","preferred":"llama",
     "active":"llama","target_port":8081,
     "backends":{"mlx":{"port":8080,"up":false},
                 "llama":{"port":8081,"up":false},
                 "ollama":{"port":11434,"up":false}}}
    """

    @Test func decodesV20Router() throws {
        let r = try JSONDecoder().decode(
            RouterStatusResponse.self, from: Self.v20.data(using: .utf8)!)
        #expect(r.preferred == "llama")
        #expect(r.backends["mlx"]?.port == 8080)
        #expect(r.uptime_secs == 0)
        #expect(r.stats.isEmpty)
    }

    @Test func decodesV27Router() throws {
        let v27 = """
        {"status":"ok","router":"lac-router v2.7","preferred":"auto",
         "active":"mlx","target_port":8080,"uptime_secs":99,"inflight":2,
         "models_mapped":3,"usage_log":"/tmp/u.jsonl",
         "backends":{"mlx":{"port":8080,"up":true}},
         "stats":{"mlx":{"port":8080,"ewma_ms":12.5,"ok":7,"err":1,"est_tokens":100}}}
        """
        let r = try JSONDecoder().decode(
            RouterStatusResponse.self, from: v27.data(using: .utf8)!)
        #expect(r.uptime_secs == 99)
        #expect(r.stats["mlx"]?.ok == 7)
        #expect(r.stats["mlx"]?.ewma_ms == 12.5)
    }
}

struct MarkdownSplitTests {
    @Test func splitsFencedCode() {
        let segs = splitMessage("hi\n```swift\nlet x = 1\n```\nbye")
        #expect(segs.count == 3)
        #expect(segs[0] == .prose("hi"))
        #expect(segs[1] == .code(language: "swift", code: "let x = 1"))
        #expect(segs[2] == .prose("bye"))
    }

    @Test func unclosedFenceBecomesCode() {
        // Mid-stream partial: remainder renders as code, never dropped.
        let segs = splitMessage("```\npartial")
        #expect(segs == [.code(language: "", code: "partial")])
    }

    @Test func plainTextIsOneProse() {
        #expect(splitMessage("just words") == [.prose("just words")])
    }

    @Test func splitsThoughtBlock() {
        let segs = splitMessage("<think>\npondering problem\n</think>\nHere is the answer")
        #expect(segs.count == 2)
        #expect(segs[0] == .thought("pondering problem"))
        #expect(segs[1] == .prose("Here is the answer"))
    }
}

struct ChatStoreTests {
    @Test func chatErrorDescriptions() {
        let err502 = ChatError.http(502, "")
        #expect(err502.errorDescription?.contains("No backend served the request (HTTP 502)") == true)
        #expect(err502.errorDescription?.contains("Start one with `lac serve mlx`.") == true)

        let err503 = ChatError.http(503, "")
        #expect(err503.errorDescription?.contains("No backend served the request (HTTP 503)") == true)

        let err500 = ChatError.http(500, "internal error")
        #expect(err500.errorDescription?.contains("Gateway HTTP 500.") == true)
    }

    @Test func chatMessageSerialization() throws {
        let msg = ChatMessage(role: "user", content: "hello world")
        let enc = try JSONEncoder().encode(msg)
        let dec = try JSONDecoder().decode(ChatMessage.self, from: enc)
        #expect(dec.role == "user")
        #expect(dec.content == "hello world")
    }

    @Test @MainActor func chatStoreMutations() {
        let store = ChatStore()
        let t = store.newThread()
        #expect(store.activeThreadId == t.id)
        let m1 = ChatMessage(role: "user", content: "Prompt 1")
        let m2 = ChatMessage(role: "assistant", content: "Response 1")
        if let idx = store.threads.firstIndex(where: { $0.id == t.id }) {
            store.threads[idx].messages.append(m1)
            store.threads[idx].messages.append(m2)
        }
        #expect(store.activeThread?.messages.count == 2)
        store.deleteMessage(id: m2.id)
        #expect(store.activeThread?.messages.count == 1)
        #expect(store.activeThread?.messages.first?.content == "Prompt 1")
    }

    @Test @MainActor func chatMessageVariants() {
        let store = ChatStore()
        let t = store.newThread()
        let v1 = MessageVariant(content: "Variant 1", model: "model-a")
        let v2 = MessageVariant(content: "Variant 2", model: "model-b")
        let msg = ChatMessage(role: "assistant", content: v1.content, model: v1.model, variants: [v1, v2], selectedVariantIndex: 0)
        if let idx = store.threads.firstIndex(where: { $0.id == t.id }) {
            store.threads[idx].messages.append(msg)
        }

        #expect(store.activeThread?.messages.last?.variants.count == 2)
        #expect(store.activeThread?.messages.last?.content == "Variant 1")

        store.selectVariant(messageId: msg.id, index: 1)
        #expect(store.activeThread?.messages.last?.selectedVariantIndex == 1)
        #expect(store.activeThread?.messages.last?.content == "Variant 2")
        #expect(store.activeThread?.messages.last?.model == "model-b")
    }

    @Test @MainActor func chatThreadForking() {
        let store = ChatStore()
        let t = store.newThread()
        let m1 = ChatMessage(role: "user", content: "Prompt 1")
        let m2 = ChatMessage(role: "assistant", content: "Answer 1")
        let m3 = ChatMessage(role: "user", content: "Prompt 2")
        let m4 = ChatMessage(role: "assistant", content: "Answer 2")
        if let idx = store.threads.firstIndex(where: { $0.id == t.id }) {
            store.threads[idx].messages.append(contentsOf: [m1, m2, m3, m4])
        }

        // Fork at m2
        let forked = store.forkThread(at: m2.id)
        #expect(forked != nil)
        #expect(store.activeThreadId == forked?.id)
        #expect(forked?.messages.count == 2)
        #expect(forked?.messages[0].content == "Prompt 1")
        #expect(forked?.messages[1].content == "Answer 1")
        #expect(forked?.title.contains("(Fork)") == true)
    }
}

struct ModelHubTests {
    @Test func installedModelFormatting() {
        let item1 = InstalledModelItem(
            id: "mlx-community/Qwen3.8-27B-4bit",
            name: "Qwen3.8-27B-4bit",
            path: "/Volumes/AIModels/Qwen3.8-27B-4bit",
            sizeBytes: Int64(16_100_000_000),
            format: "MLX",
            sourceDir: "/Volumes/AIModels",
            modifiedDate: Date()
        )
        #expect(item1.formattedSize.contains("GB"))
        #expect(item1.ramRecommendation.label.contains("Balanced"))

        let item2 = InstalledModelItem(
            id: "small-coder.gguf",
            name: "small-coder.gguf",
            path: "/tmp/small-coder.gguf",
            sizeBytes: Int64(450_000_000),
            format: "GGUF",
            sourceDir: "/tmp",
            modifiedDate: Date()
        )
        #expect(item2.formattedSize.contains("MB"))
        #expect(item2.ramRecommendation.label.contains("Lightweight"))
    }
}

struct CodeDiffTests {
    @Test func lineDiffAndHunkPartitioning() {
        let orig = "fn main() {\n    println!(\"old\");\n    let x = 1;\n    let y = 2;\n}"
        let mod = "fn main() {\n    println!(\"new\");\n    let x = 10;\n    let y = 2;\n}"
        let lines = computeLineDiff(original: orig, modified: mod)
        #expect(!lines.isEmpty)
        let hunks = partitionIntoHunks(diffLines: lines)
        #expect(!hunks.isEmpty)
        #expect(hunks[0].addedCount > 0)
        #expect(hunks[0].removedCount > 0)

        // All accepted -> should equal modified
        var acceptedHunks = hunks
        for i in acceptedHunks.indices { acceptedHunks[i].state = .accepted }
        let codeAccepted = synthesizeCodeFromHunks(diffLines: lines, hunks: acceptedHunks)
        #expect(codeAccepted == mod)

        // All rejected -> should equal original
        var rejectedHunks = hunks
        for i in rejectedHunks.indices { rejectedHunks[i].state = .rejected }
        let codeRejected = synthesizeCodeFromHunks(diffLines: lines, hunks: rejectedHunks)
        #expect(codeRejected == orig)
    }
}

struct LoopsStoreTests {
    @Test func taskItemAndPriorities() {
        let task = TaskItem(
            id: "test-001",
            task: "Verify local router TTFT metrics",
            priority: .critical,
            status: .pending,
            loopName: "daily-coding.yaml",
            module: "lac-router"
        )
        #expect(task.id == "test-001")
        #expect(task.priority == .critical)
        #expect(task.priority.label == "Critical")
        #expect(task.status == .pending)
        #expect(task.status.title == "Queue")
        #expect(task.loopName == "daily-coding.yaml")
    }

    @Test @MainActor func loopsStoreMutations() {
        let store = LoopsStore()
        #expect(!store.loops.isEmpty)
        #expect(!store.tasks.isEmpty)

        // Add task
        let newTask = TaskItem(
            id: "task-unit-test",
            task: "Test autonomous pipeline execution",
            priority: .high,
            status: .pending
        )
        store.addTask(newTask)
        #expect(store.tasks.contains(where: { $0.id == "task-unit-test" }))

        // Transition task status
        store.moveTask(id: "task-unit-test", to: .inProgress)
        #expect(store.tasks.first(where: { $0.id == "task-unit-test" })?.status == .inProgress)

        store.moveTask(id: "task-unit-test", to: .reviewGate)
        #expect(store.tasks.first(where: { $0.id == "task-unit-test" })?.status == .reviewGate)

        store.moveTask(id: "task-unit-test", to: .completed)
        #expect(store.tasks.first(where: { $0.id == "task-unit-test" })?.status == .completed)

        // Delete task
        store.deleteTask(id: "task-unit-test")
        #expect(!store.tasks.contains(where: { $0.id == "task-unit-test" }))
    }
}

struct CodeAssistantWorkingSetTests {
    @Test func languageDetection() {
        #expect(CodeAssistantStore.language(for: URL(fileURLWithPath: "/tmp/main.rs")) == "Rust")
        #expect(CodeAssistantStore.language(for: URL(fileURLWithPath: "/tmp/App.swift")) == "Swift")
        #expect(CodeAssistantStore.language(for: URL(fileURLWithPath: "/tmp/script.py")) == "Python")
        #expect(CodeAssistantStore.language(for: URL(fileURLWithPath: "/tmp/web.ts")) == "TypeScript")
        #expect(CodeAssistantStore.language(for: URL(fileURLWithPath: "/tmp/tool.go")) == "Go")
        #expect(CodeAssistantStore.language(for: URL(fileURLWithPath: "/tmp/build.sh")) == "Shell")
    }

    @Test @MainActor func editorTabManagement() {
        let store = CodeAssistantStore()
        #expect(store.tabs.count == 1)
        #expect(store.tabs[0].title == "RingBuffer.rs")
        #expect(store.activeTabId == store.tabs[0].id)

        // Open a file
        let fileUrl = URL(fileURLWithPath: "/tmp/worker.rs")
        store.openFile(url: fileUrl, content: "pub fn worker() {}")
        #expect(store.tabs.count == 2)
        #expect(store.activeTab?.title == "worker.rs")
        #expect(store.selectedLanguage == "Rust")
        #expect(store.sourceCode == "pub fn worker() {}")

        // Edit source code -> marks active tab modified
        store.sourceCode = "pub fn worker() { println!(); }"
        #expect(store.activeTab?.isModified == true)

        // New tab
        store.newTab()
        #expect(store.tabs.count == 3)
        #expect(store.activeTab?.title.hasPrefix("Untitled") == true)

        // Close tab
        let untitledId = store.activeTabId
        store.closeTab(id: untitledId)
        #expect(store.tabs.count == 2)
        #expect(store.activeTab?.id != untitledId)
    }

    @Test @MainActor func closeOtherAndAllTabs() {
        let store = CodeAssistantStore()
        let url1 = URL(fileURLWithPath: "/tmp/a.rs")
        let url2 = URL(fileURLWithPath: "/tmp/b.rs")
        let url3 = URL(fileURLWithPath: "/tmp/c.rs")

        store.openFile(url: url1, content: "// a")
        store.openFile(url: url2, content: "// b")
        store.openFile(url: url3, content: "// c")
        #expect(store.tabs.count == 4)

        let targetId = store.tabs.first(where: { $0.title == "b.rs" })!.id
        store.closeOtherTabs(id: targetId)
        #expect(store.tabs.count == 1)
        #expect(store.activeTabId == targetId)
        #expect(store.activeTab?.title == "b.rs")

        store.closeAllTabs()
        #expect(store.tabs.count == 1)
        #expect(store.activeTab?.title.hasPrefix("Untitled") == true)
    }

    @Test @MainActor func saveAllTabs() {
        let store = CodeAssistantStore()
        let tempDir = FileManager.default.temporaryDirectory
        let fileA = tempDir.appendingPathComponent("test_save_a_\(UUID().uuidString).rs")
        let fileB = tempDir.appendingPathComponent("test_save_b_\(UUID().uuidString).rs")

        try? "initial a".write(to: fileA, atomically: true, encoding: .utf8)
        try? "initial b".write(to: fileB, atomically: true, encoding: .utf8)

        defer {
            try? FileManager.default.removeItem(at: fileA)
            try? FileManager.default.removeItem(at: fileB)
        }

        store.openFile(url: fileA, content: "initial a")
        store.openFile(url: fileB, content: "initial b")

        // Mutate both
        if let idxA = store.tabs.firstIndex(where: { $0.url == fileA }) {
            store.tabs[idxA].content = "updated a"
            store.tabs[idxA].isModified = true
        }
        if let idxB = store.tabs.firstIndex(where: { $0.url == fileB }) {
            store.tabs[idxB].content = "updated b"
            store.tabs[idxB].isModified = true
        }

        #expect(store.tabs.filter { $0.isModified }.count == 2)
        store.saveAllTabs()
        #expect(store.tabs.filter { $0.isModified }.count == 0)

        let diskA = try? String(contentsOf: fileA, encoding: .utf8)
        let diskB = try? String(contentsOf: fileB, encoding: .utf8)
        #expect(diskA == "updated a")
        #expect(diskB == "updated b")
    }
}

struct AgentConsoleStoreTests {
    @Test @MainActor func consoleStateAndClear() {
        let console = AgentConsoleStore()
        #expect(console.output.isEmpty)
        #expect(!console.isRunning)
        #expect(console.lastExitCode == nil)
        #expect(console.activeCommand == nil)
        #expect(console.executionDuration == 0)

        console.output = "some log"
        console.lastExitCode = 0
        console.activeCommand = "test"
        console.clear()

        #expect(console.output.isEmpty)
        #expect(console.lastExitCode == nil)
        #expect(console.activeCommand == nil)
    }

    @Test @MainActor func consoleProcessExecution() async {
        let console = AgentConsoleStore()
        let tmpDir = FileManager.default.temporaryDirectory

        console.execute(
            command: "/bin/echo",
            arguments: ["hello lac test suite"],
            workingDirectory: tmpDir,
            title: "echo test"
        )

        // Wait briefly for process to run and terminate
        for _ in 0..<30 {
            if !console.isRunning && console.lastExitCode != nil {
                break
            }
            try? await Task.sleep(nanoseconds: 100_000_000)
        }

        #expect(!console.isRunning)
        #expect(console.lastExitCode == 0)
        #expect(console.output.contains("hello lac test suite"))
        #expect(console.output.contains("code 0"))
    }
}

struct WorkspaceFileTreeTests {
    @Test func gitStatusParsing() {
        let root = URL(fileURLWithPath: "/workspace")
        let gitOutput = """
         M rust-src/src/lac.rs
        ?? SwiftUI/Sources/LoopLACStudio/NewFile.swift
         A tests/gate.rs
         D obsolete.txt
        """
        let parsed = WorkspaceFileTree.parseGitStatusOutput(gitOutput, rootUrl: root)
        #expect(parsed["/workspace/rust-src/src/lac.rs"] == .modified)
        #expect(parsed["/workspace/SwiftUI/Sources/LoopLACStudio/NewFile.swift"] == .untracked)
        #expect(parsed["/workspace/tests/gate.rs"] == .added)
        #expect(parsed["/workspace/obsolete.txt"] == .deleted)
    }

    @Test func scanFlatFilesFindsChildren() {
        let tempDir = FileManager.default.temporaryDirectory.appendingPathComponent("lac_test_scan_\(UUID().uuidString)")
        try? FileManager.default.createDirectory(at: tempDir, withIntermediateDirectories: true)
        defer {
            try? FileManager.default.removeItem(at: tempDir)
        }

        let file1 = tempDir.appendingPathComponent("worker.rs")
        let subDir = tempDir.appendingPathComponent("sub")
        try? FileManager.default.createDirectory(at: subDir, withIntermediateDirectories: true)
        let file2 = subDir.appendingPathComponent("app.swift")

        try? "fn worker() {}".write(to: file1, atomically: true, encoding: .utf8)
        try? "import SwiftUI".write(to: file2, atomically: true, encoding: .utf8)

        let scanned = WorkspaceFileTree.scanFlatFiles(url: tempDir, maxDepth: 3)
        #expect(scanned.count == 2)
        #expect(scanned.contains(where: { $0.name == "worker.rs" }))
        #expect(scanned.contains(where: { $0.name == "app.swift" }))
    }
}

struct CommandPaletteTests {
    @Test func paletteItemPropertiesAndEquality() {
        let item1 = PaletteItem(
            title: "Quick Open File",
            subtitle: "Spotlight file search across workspace",
            icon: "doc.text.magnifyingglass",
            category: "Actions",
            shortcut: "⌘P",
            action: {}
        )
        #expect(item1.title == "Quick Open File")
        #expect(item1.shortcut == "⌘P")
        #expect(item1.category == "Actions")
        #expect(item1.icon == "doc.text.magnifyingglass")

        // Self equality
        #expect(item1 == item1)

        let item2 = PaletteItem(
            title: "Run Swift Tests",
            subtitle: nil,
            icon: "swift",
            category: "Actions",
            shortcut: nil,
            action: {}
        )
        #expect(item1 != item2)
        #expect(item2.subtitle == nil)
    }
}

struct HFRepoFileItemTests {
    @Test func decodesFromTreeApiJson() throws {
        let json = """
        [
            {"path": "Qwen3.8-27B-Q4_K_M.gguf", "size": 17179869184, "type": "file"},
            {"path": "Qwen3.8-27B-Q8_0.gguf", "size": 28991029248, "type": "file"},
            {"path": "model.safetensors", "size": 52428800, "type": "file"},
            {"path": "tokenizer.json", "size": 1048576, "type": "file"}
        ]
        """
        let items = try JSONDecoder().decode([HFRepoFileItem].self, from: json.data(using: .utf8)!)
        #expect(items.count == 4)

        // Item 0: Q4_K_M GGUF
        #expect(items[0].fileName == "Qwen3.8-27B-Q4_K_M.gguf")
        #expect(items[0].isGguf == true)
        #expect(items[0].isSafetensors == false)
        #expect(items[0].quantizationTag == "Q4_K_M")
        #expect(items[0].formattedSize == "16.00 GB")

        // Item 1: Q8_0 GGUF
        #expect(items[1].quantizationTag == "Q8_0")
        #expect(items[1].isGguf == true)

        // Item 2: Safetensors
        #expect(items[2].isSafetensors == true)
        #expect(items[2].quantizationTag == "SafeTensors")
        #expect(items[2].formattedSize == "50.0 MB")

        // Item 3: Generic file
        #expect(items[3].quantizationTag == "Model File")
        #expect(items[3].formattedSize == "1.0 MB")
    }

    @Test func handlesZeroOrNilSizes() {
        let zeroItem = HFRepoFileItem(path: "empty.bin", size: 0, type: "file")
        #expect(zeroItem.formattedSize == "—")

        let nilItem = HFRepoFileItem(path: "folder", size: nil, type: "directory")
        #expect(nilItem.formattedSize == "—")
    }
}

struct LoopYamlSerializationTests {
    @Test func parseAndSerializeRoundTrip() {
        let original = [
            TaskItem(
                id: "lac-test-01",
                task: "Audit MLX cache hit rate",
                priority: .critical,
                status: .inProgress,
                loopName: "daily-coding.yaml",
                created: "2026-09-13T12:00:00Z",
                module: "rust-src/kv"
            ),
            TaskItem(
                id: "lac-test-02",
                task: "Refactor Liquid Glass modal sheets",
                priority: .low,
                status: .completed,
                loopName: "bug-fix.yaml",
                created: "2026-09-13T14:30:00Z",
                module: "SwiftUI/Glass"
            )
        ]

        let yaml = LoopsStore.serializeTasksYaml(original)
        #expect(yaml.contains("lac-test-01"))
        #expect(yaml.contains("Audit MLX cache hit rate"))
        #expect(yaml.contains("priority: critical"))
        #expect(yaml.contains("status: in_progress"))
        #expect(yaml.contains("module: \"rust-src/kv\""))

        let parsed = LoopsStore.parseTasksYaml(yaml)
        #expect(parsed.count == 2)
        #expect(parsed[0].id == "lac-test-01")
        #expect(parsed[0].task == "Audit MLX cache hit rate")
        #expect(parsed[0].priority == .critical)
        #expect(parsed[0].status == .inProgress)
        #expect(parsed[0].module == "rust-src/kv")

        #expect(parsed[1].id == "lac-test-02")
        #expect(parsed[1].priority == .low)
        #expect(parsed[1].status == .completed)
        #expect(parsed[1].module == "SwiftUI/Glass")
    }
}

struct CodeAssistantTabTests {
    @Test @MainActor func tabManagementAndModifications() {
        let store = CodeAssistantStore()
        #expect(store.tabs.count == 1)
        #expect(store.tabs.first?.title == "RingBuffer.rs")

        let fileUrl = URL(fileURLWithPath: "/workspace/rust-src/src/lac.rs")
        store.openFile(url: fileUrl, content: "fn main() {}")
        #expect(store.tabs.count == 2)
        #expect(store.activeTab?.title == "lac.rs")
        #expect(store.sourceCode == "fn main() {}")
        #expect(store.selectedLanguage == "Rust")

        // Modifying code marks tab as modified
        store.sourceCode = "fn main() { println!(\"hello\"); }"
        #expect(store.activeTab?.isModified == true)

        // Undo apply
        store.applyToEditor("fn refactored() {}")
        #expect(store.sourceCode == "fn refactored() {}")
        #expect(store.lastAppliedCode == "fn main() { println!(\"hello\"); }")

        store.undoApply()
        #expect(store.sourceCode == "fn main() { println!(\"hello\"); }")
        #expect(store.lastAppliedCode == nil)
    }
}

struct ModelHubFilterTests {
    private func item(_ id: String) -> HFModelItem {
        HFModelItem(id: id, downloads: nil, likes: nil, tags: nil, pipeline_tag: nil, createdAt: nil)
    }

    @Test func sizeBucketsCoverCommonParams() {
        typealias Size = ModelHubStore.ModelSizeFilter
        #expect(Size.small.matches(item: item("mlx-community/Qwen3-1.7B-4bit")))
        #expect(Size.small.matches(item: item("mlx-community/Llama-3.2-1B-Instruct-4bit")))
        #expect(Size.medium.matches(item: item("mlx-community/Qwen3.8-27B-4bit")))
        #expect(Size.medium.matches(item: item("mlx-community/Qwen2.5-Coder-32B-Instruct-4bit")))
        #expect(Size.large.matches(item: item("unsloth/Llama-3.3-70B-GGUF")))
        #expect(!Size.small.matches(item: item("mlx-community/Qwen3.8-27B-4bit")))
        #expect(!Size.large.matches(item: item("mlx-community/Qwen3.8-27B-4bit")))
        #expect(Size.medium.matches(item: item("mistralai/Mixtral-8x7B-Instruct-v0.1")))
        #expect(Size.small.matches(item: item("HuggingFaceTB/SmolLM2-1.7B-Instruct")))
        #expect(Size.all.matches(item: item("anything/AtAll-999B")))
    }

    @Test func decodesSparseSearchPayload() throws {        // HF occasionally omits keys; discovery must not blank on that.
        let json = """
        [{"id":"mlx-community/Qwen3.8-27B-4bit","downloads":12}]
        """
        let items = try JSONDecoder().decode([HFModelItem].self, from: Data(json.utf8))
        #expect(items.count == 1)
        #expect(items[0].likes == nil)
        #expect(items[0].tags == nil)
        #expect(items[0].author == "mlx-community")
        #expect(items[0].modelName == "Qwen3.8-27B-4bit")
    }

    @Test func ramEstimatorMath() {
        // 27B Q4 ≈ 16 GB weights, ~26 GB with KV/OS headroom.
        let qwen27 = item("mlx-community/Qwen3.8-27B-4bit")
        #expect(qwen27.ramFitEstimate(hostRamGB: 16).color == .red)
        #expect(qwen27.ramFitEstimate(hostRamGB: 128).color == .green)
        #expect(qwen27.ramFitEstimate(hostRamGB: nil).label.contains("16"))
        // 1B Q4 fits anywhere.
        let tiny = item("mlx-community/Llama-3.2-1B-Instruct-4bit")
        #expect(tiny.ramFitEstimate(hostRamGB: 16).color == .green)
        // No parseable size → neutral, never a false verdict.
        #expect(item("someorg/mystery-model").ramFitEstimate(hostRamGB: 16).color == .blue)
    }

    @Test func quantizationDetectionCoversGgufIds() {
        func tagged(_ id: String, _ tags: [String]) -> HFModelItem {
            HFModelItem(id: id, downloads: nil, likes: nil, tags: tags, pipeline_tag: nil, createdAt: nil)
        }
        // Reviewer case: tag-less 70B GGUF must not estimate at 4-bit.
        let gguf70 = item("unsloth/Llama-3.3-70B-Q8_0-GGUF")
        #expect(gguf70.quantization == "8-bit")
        #expect(gguf70.ramFitEstimate(hostRamGB: 64).color == .red)
        #expect(tagged("org/Model-32B", ["q5_k_m"]).quantization == "5-bit")
        #expect(tagged("org/Model-14B", ["bf16"]).quantization == "FP16")
        #expect(item("mlx-community/Qwen3.8-27B-4bit").quantization == "4-bit")
    }
}

struct CompletionBudgetTests {
    @Test func maxTokensNeverExceeds4k() {
        // 64k context / 2 would OOM a local 27B — hard cap at 4096.
        #expect(ChatStore.maxTokens(forContextCap: 65536) == 4096)
        #expect(ChatStore.maxTokens(forContextCap: 32768) == 4096)
        #expect(ChatStore.maxTokens(forContextCap: 4096) == 2048)
        #expect(ChatStore.maxTokens(forContextCap: 0) == nil)
        #expect(ChatStore.maxTokens(forContextCap: -1) == nil)
    }
}

struct ThreadTitlePersistenceTests {
    @Test @MainActor func renameSurvivesReload() {
        let store = ChatStore()
        let t = store.newThread()
        if let idx = store.threads.firstIndex(where: { $0.id == t.id }) {
            store.threads[idx].messages.append(
                ChatMessage(role: "user", content: "derivable title seed"))
        }
        store.renameThread(t.id, title: "My Custom Title")

        // A fresh instance (simulated restart) must keep the rename.
        let reloaded = ChatStore()
        #expect(reloaded.threads.first(where: { $0.id == t.id })?.title == "My Custom Title")

        // Cleanup: remove the probe thread from both files.
        store.deleteThread(t.id)
        reloaded.deleteThread(t.id)
    }
}

struct LoopsWritePathTests {
    @Test @MainActor func writesTargetUserQueueNeverTemplates() {
        let store = LoopsStore()
        let url = store.tasksWriteURL()
        #expect(url.path.hasSuffix("todo/lac-tasks.yaml"))
        #expect(!url.path.contains("templates"))
    }
}

struct PaletteConsoleBridgeTests {
    @Test @MainActor func pendingCommandRoundTrip() {
        let store = CodeAssistantStore()
        #expect(store.pendingConsoleCommand == nil)
        store.pendingConsoleCommand = .cargoTests
        #expect(store.pendingConsoleCommand == .cargoTests)
        store.pendingConsoleCommand = nil
        #expect(store.pendingConsoleCommand == nil)
    }
}



