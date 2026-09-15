import Foundation
import SwiftUI

// MARK: - Hugging Face Model Item (LM Studio style model discovery)

public struct HFModelItem: Identifiable, Codable, Hashable {
    public let id: String
    public let downloads: Int?
    public let likes: Int?
    public let tags: [String]?
    public let pipeline_tag: String?
    public let createdAt: String?

    enum CodingKeys: String, CodingKey {
        case id, downloads, likes, tags, pipeline_tag, createdAt
    }

    public init(id: String, downloads: Int? = nil, likes: Int? = nil, tags: [String]? = nil, pipeline_tag: String? = nil, createdAt: String? = nil) {
        self.id = id
        self.downloads = downloads
        self.likes = likes
        self.tags = tags
        self.pipeline_tag = pipeline_tag
        self.createdAt = createdAt
    }

    /// Tolerant decode: one malformed entry (e.g. downloads as String,
    /// tags as mixed array) must not fail the whole 30-item page.
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        downloads = try? c.decodeIfPresent(Int.self, forKey: .downloads)
        likes = try? c.decodeIfPresent(Int.self, forKey: .likes)
        tags = try? c.decodeIfPresent([String].self, forKey: .tags)
        pipeline_tag = try? c.decodeIfPresent(String.self, forKey: .pipeline_tag)
        createdAt = try? c.decodeIfPresent(String.self, forKey: .createdAt)
    }

    public var author: String {
        let parts = id.split(separator: "/")
        if parts.count > 1 { return String(parts[0]) }
        return "huggingface"
    }

    public var modelName: String {
        let parts = id.split(separator: "/")
        if parts.count > 1 { return String(parts[1]) }
        return id
    }

    public var downloadFormatted: String {
        guard let d = downloads else { return "—" }
        if d >= 1_000_000 {
            return String(format: "%.1fM", Double(d) / 1_000_000.0)
        } else if d >= 1_000 {
            return String(format: "%.1fK", Double(d) / 1_000.0)
        }
        return "\(d)"
    }

    public var quantization: String {
        func detect(in s: String) -> String? {
            let lower = s.lowercased()
            if lower.contains("3-bit") || lower.contains("3bit") || lower.contains("q3") { return "3-bit" }
            if lower.contains("4-bit") || lower.contains("4bit") || lower.contains("q4") { return "4-bit" }
            if lower.contains("5-bit") || lower.contains("5bit") || lower.contains("q5") { return "5-bit" }
            if lower.contains("6-bit") || lower.contains("6bit") || lower.contains("q6") { return "6-bit" }
            if lower.contains("8-bit") || lower.contains("8bit") || lower.contains("q8") { return "8-bit" }
            if lower.contains("fp16") || lower.contains("bf16") || lower.contains("f16") { return "FP16" }
            return nil
        }
        if let tags = tags {
            for tag in tags {
                if let hit = detect(in: tag) { return hit }
            }
        }
        if let hit = detect(in: id) { return hit }
        return "MLX"
    }

    public var isMlx: Bool {
        tags?.contains("mlx") == true || id.lowercased().contains("mlx")
    }

    public var isGguf: Bool {
        tags?.contains("gguf") == true || id.lowercased().contains("gguf")
    }

    /// Weight bytes per parameter by quantization (weights only).
    public var bytesPerParam: Double {
        switch quantization {
        case "3-bit": return 0.45
        case "4-bit": return 0.6
        case "5-bit": return 0.75
        case "6-bit": return 0.85
        case "8-bit": return 1.1
        case "FP16": return 2.1
        default: return 0.6 // MLX default is 4-bit
        }
    }

    /// Weight math (params × quant) plus KV/OS headroom, checked against
    /// the host. Matches the backend gate in pull_models.rs (27B wants
    /// 32GB+ headroom). Nil host → weight-only label, no verdict.
    public func ramFitEstimate(hostRamGB: Double?) -> (label: String, color: Color) {
        guard let params = ModelHubStore.ModelSizeFilter.paramBillions(in: id.lowercased()) else {
            return ("Apple Silicon Compatible", .blue)
        }
        let weightsGB = params * bytesPerParam
        let needGB = weightsGB * 1.35 + 4.0
        func gb(_ v: Double) -> String { String(Int(v.rounded())) }
        guard let host = hostRamGB, host > 0 else {
            return ("~\(gb(weightsGB)) GB weights", .blue)
        }
        if needGB > host {
            return ("Needs ~\(gb(needGB)) GB RAM (host \(gb(host)) GB)", .red)
        } else if needGB > host * 0.7 {
            return ("Tight on \(gb(host)) GB host (~\(gb(needGB)) GB)", .orange)
        }
        return ("Fits \(gb(host)) GB host (~\(gb(weightsGB)) GB weights)", .green)
    }
}

// MARK: - Hugging Face Repo File Item (LM Studio file inspection & quantization picker)

public struct HFRepoFileItem: Identifiable, Codable, Hashable {
    public var id: String { path }
    public let path: String
    public let size: Int64?
    public let type: String?

    enum CodingKeys: String, CodingKey { case path, size, type }

    public init(path: String, size: Int64? = nil, type: String? = nil) {
        self.path = path
        self.size = size
        self.type = type
    }

    /// Tolerant decode: entries without a path are skipped by the caller
    /// via Failable wrapper instead of failing the whole manifest.
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        path = try c.decode(String.self, forKey: .path)
        size = try? c.decodeIfPresent(Int64.self, forKey: .size)
        type = try? c.decodeIfPresent(String.self, forKey: .type)
    }

    public var fileName: String {
        path.split(separator: "/").last.map(String.init) ?? path
    }

    public var isGguf: Bool {
        path.lowercased().hasSuffix(".gguf")
    }

    public var isSafetensors: Bool {
        path.lowercased().hasSuffix(".safetensors")
    }

    public var formattedSize: String {
        guard let s = size, s > 0 else { return "—" }
        let gb = Double(s) / (1024.0 * 1024.0 * 1024.0)
        if gb >= 1.0 {
            return String(format: "%.2f GB", gb)
        }
        let mb = Double(s) / (1024.0 * 1024.0)
        return String(format: "%.1f MB", mb)
    }

    public var quantizationTag: String {
        let upper = fileName.uppercased()
        for q in ["Q4_K_M", "Q4_K_S", "Q4_0", "Q4_1", "Q5_K_M", "Q5_K_S", "Q5_0", "Q8_0", "Q6_K", "Q3_K_M", "Q2_K", "FP16", "BF16"] {
            if upper.contains(q) { return q }
        }
        if isGguf { return "GGUF" }
        if isSafetensors { return "SafeTensors" }
        return "Model File"
    }
}

// MARK: - Installed Model Item (LM Studio Local Library)

public struct InstalledModelItem: Identifiable, Hashable {
    public let id: String
    public let name: String
    public let path: String
    public let sizeBytes: Int64
    public let format: String
    public let sourceDir: String
    public let modifiedDate: Date

    public var formattedSize: String {
        let gb = Double(sizeBytes) / (1024.0 * 1024.0 * 1024.0)
        if gb >= 1.0 {
            return String(format: "%.1f GB", gb)
        }
        let mb = Double(sizeBytes) / (1024.0 * 1024.0)
        return String(format: "%.0f MB", mb)
    }

    public var ramRecommendation: (label: String, color: Color) {
        let gb = Double(sizeBytes) / (1024.0 * 1024.0 * 1024.0)
        if gb >= 28.0 {
            return ("64GB+ RAM (Heavy)", .red)
        } else if gb >= 12.0 {
            return ("16GB–32GB RAM (Balanced)", .green)
        } else {
            return ("8GB+ RAM (Lightweight)", .green)
        }
    }
}

// MARK: - Model Hub Store: Query Hugging Face API like LM Studio

@MainActor
public class ModelHubStore: ObservableObject {
    public enum HubTab: String, CaseIterable, Identifiable {
        case discover = "Discover (Hugging Face)"
        case installed = "Installed Models"
        public var id: String { rawValue }
    }

    @Published public var hubTab: HubTab = .discover
    @Published public var models: [HFModelItem] = []
    @Published public var installedModels: [InstalledModelItem] = []
    @Published public var totalInstalledBytes: Int64 = 0
    @Published public var freeDiskBytes: Int64 = 0
    @Published public var isScanningInstalled: Bool = false
    @Published public var searchQuery: String = ""
    @Published public var selectedFilter: ModelFilter = .appleSilicon
    @Published public var selectedSize: ModelSizeFilter = .all
    @Published public var isLoading: Bool = false
    @Published public var errorMessage: String? = nil
    @Published public var inspectingModel: HFModelItem? = nil
    @Published public var repoFiles: [HFRepoFileItem] = []
    @Published public var isLoadingRepoFiles: Bool = false
    @Published public var repoFilesError: String? = nil

    public enum ModelFilter: String, CaseIterable, Identifiable {
        case appleSilicon = "Apple Silicon"
        case mlx = "MLX Native"
        case gguf = "GGUF (llama.cpp)"
        case coding = "Coding"
        case reasoning = "Reasoning"
        case vision = "Vision & Multimodal"

        public var id: String { rawValue }
    }

    public enum ModelSizeFilter: String, CaseIterable, Identifiable {
        case all = "All Sizes"
        case small = "≤ 13B (Fast)"
        case medium = "14B – 69B (Balanced)"
        case large = "70B+ (Heavy)"

        public var id: String { rawValue }

        public func matches(item: HFModelItem) -> Bool {
            let lower = item.id.lowercased()
            if self == .all { return true }
            // Numeric parse: "27b" contains "7b", so substring lists
            // mis-bucket (e.g. Qwen3.8-27B matched "7b" → small).
            if let params = Self.paramBillions(in: lower) {
                switch self {
                case .small: return params <= 13
                case .medium: return params > 13 && params < 70
                case .large: return params >= 70
                case .all: return true
                }
            }
            // No parseable parameter count: only match obvious tiny aliases.
            if self == .small {
                return lower.contains("lite") || lower.contains("mini") || lower.contains("tiny")
            }
            return false
        }

        /// Parse "27b" / "1.5b" / MoE totals like "8x7b" from a repo id.
        /// Returns nil when no parameter count is present.
        public static func paramBillions(in lowercasedId: String) -> Double? {
            let id = lowercasedId
            if let m = id.firstMatch(of: /(\d+(?:\.\d+)?)\s*x\s*(\d+(?:\.\d+)?)\s*b\b/) {
                if let experts = Double(m.1), let per = Double(m.2) { return experts * per }
            }
            if let m = id.firstMatch(of: /(\d+(?:\.\d+)?)\s*b\b/) {
                return Double(m.1)
            }
            return nil
        }
    }

    public var filteredModels: [HFModelItem] {
        if selectedSize == .all {
            return models
        }
        return models.filter { selectedSize.matches(item: $0) }
    }

    public static func openInBrowser(modelId: String) {
        if let url = URL(string: "https://huggingface.co/\(modelId)") {
            NSWorkspace.shared.open(url)
        }
    }

    /// Best-known download size for a model: the inspected file manifest
    /// total when it belongs to this model, else nil (unknown → the pull
    /// gate intentionally fails OPEN: the grid has no per-repo sizes
    /// without an extra API round-trip per card, so unknown sizes skip
    /// the disk check. Inspect Files first for an exact gated pull.
    public func estimatedBytes(for item: HFModelItem) -> Int64? {
        guard inspectingModel?.id == item.id, !repoFiles.isEmpty else { return nil }
        let total = repoFiles.compactMap(\.size).reduce(0, +)
        return total > 0 ? total : nil
    }

    private var searchTask: Task<Void, Never>?
    /// Monotonic fetch generation: only the latest search may publish
    /// state, so rapid filter taps and debounced queries cannot resolve
    /// out of order or leave a cancelled task's spinner stuck.
    private var fetchSeq = 0
    /// Monotonic repo-inspect generation: rapid Inspect A-then-B cannot
    /// let A's manifest overwrite B's sheet.
    private var inspectSeq = 0
    /// Page size for HF search (LM Studio style). Load More grows it.
    public var resultLimit = 30
    public var hasMoreResults = true

    // Curated high-performance starter models if network is cold
    public static let curatedTopModels: [HFModelItem] = [
        HFModelItem(
            id: "lmstudio-community/Qwen3.8-27B-MLX-4bit",
            downloads: 4564350,
            likes: 46,
            tags: ["mlx", "4-bit", "conversational", "text-generation"],
            pipeline_tag: "text-generation",
            createdAt: "2026-08-14T15:08:24.000Z"
        ),
        HFModelItem(
            id: "mlx-community/Qwen3.8-27B-4bit",
            downloads: 2890120,
            likes: 82,
            tags: ["mlx", "4-bit", "conversational"],
            pipeline_tag: "text-generation",
            createdAt: "2026-08-10T10:00:00.000Z"
        ),
        HFModelItem(
            id: "mlx-community/Qwen2.5-Coder-32B-Instruct-4bit",
            downloads: 3840200,
            likes: 195,
            tags: ["mlx", "4-bit", "code", "text-generation"],
            pipeline_tag: "text-generation",
            createdAt: "2026-07-20T12:00:00.000Z"
        ),
        HFModelItem(
            id: "mlx-community/DeepSeek-Coder-V2-Lite-Instruct-4bit",
            downloads: 1980400,
            likes: 114,
            tags: ["mlx", "4-bit", "code"],
            pipeline_tag: "text-generation",
            createdAt: "2026-06-15T08:00:00.000Z"
        ),
        HFModelItem(
            id: "lmstudio-community/Qwen3.8-27B-MLX-8bit",
            downloads: 4321838,
            likes: 24,
            tags: ["mlx", "8-bit", "conversational"],
            pipeline_tag: "text-generation",
            createdAt: "2026-08-14T16:00:00.000Z"
        ),
        HFModelItem(
            id: "mlx-community/Meta-Llama-3.1-8B-Instruct-4bit",
            downloads: 5120900,
            likes: 310,
            tags: ["mlx", "4-bit", "text-generation"],
            pipeline_tag: "text-generation",
            createdAt: "2026-07-25T14:00:00.000Z"
        ),
        HFModelItem(
            id: "unsloth/Qwen3.6-27B-MTP-GGUF",
            downloads: 1420100,
            likes: 64,
            tags: ["gguf", "q8_0", "text-generation"],
            pipeline_tag: "text-generation",
            createdAt: "2026-08-01T10:00:00.000Z"
        )
    ]

    public init() {
        self.models = Self.curatedTopModels
        searchTask = Task {
            await fetchTopModels()
        }
        // Defer the installed-model disk walk past first paint so the Hub
        // renders instantly from the curated list, then backfills local state.
        // The walk itself runs off the main thread (see scanInstalledModels).
        Task {
            try? await Task.sleep(nanoseconds: 500_000_000)
            if !Task.isCancelled { scanInstalledModels() }
        }
    }

    // MARK: - On-Device Model Discovery (LM Studio style)

    /// Disk walk runs on a background queue; only the publish hops to MainActor.
    public func scanInstalledModels() {
        isScanningInstalled = true
        Task.detached(priority: .utility) { [weak self] in
            let found = Self.walkInstalledModels()
            let freeDisk: Int64 = {
                let home = NSHomeDirectory()
                if let values = try? URL(fileURLWithPath: home).resourceValues(forKeys: [.volumeAvailableCapacityForImportantUsageKey, .volumeAvailableCapacityKey]) {
                    return values.volumeAvailableCapacityForImportantUsage ?? Int64(values.volumeAvailableCapacity ?? 0)
                }
                return 0
            }()
            await MainActor.run {
                guard let self else { return }
                self.freeDiskBytes = freeDisk
                self.installedModels = found.sorted { $0.sizeBytes > $1.sizeBytes }
                self.totalInstalledBytes = found.reduce(0) { $0 + $1.sizeBytes }
                self.isScanningInstalled = false
            }
        }
    }

    /// Synchronous directory walk. Called from a detached task only — never on MainActor.
    nonisolated private static func walkInstalledModels() -> [InstalledModelItem] {
        var found: [InstalledModelItem] = []
        let fm = FileManager.default
        let home = NSHomeDirectory()

        let searchDirs = [
            "/Volumes/AIModels",
            home + "/.lac/models",
            home + "/.cache/huggingface/hub",
            home + "/.ollama/models/manifests"
        ]

        for base in searchDirs {
            guard fm.fileExists(atPath: base) else { continue }
            guard let contents = try? fm.contentsOfDirectory(atPath: base) else { continue }

            for item in contents {
                if item.hasPrefix(".") { continue }
                let itemPath = (base as NSString).appendingPathComponent(item)
                var isDir: ObjCBool = false
                guard fm.fileExists(atPath: itemPath, isDirectory: &isDir) else { continue }

                if isDir.boolValue {
                    let size = directorySize(at: itemPath)
                    if size > 10 * 1024 * 1024 { // at least 10MB
                        let isMlx = item.lowercased().contains("mlx") || base.contains("mlx")
                        let isHf = base.contains("huggingface")
                        let format = isMlx ? "MLX" : (isHf ? "Hugging Face" : "Model Dir")
                        let attrs = try? fm.attributesOfItem(atPath: itemPath)
                        let mod = (attrs?[.modificationDate] as? Date) ?? Date()

                        var cleanName = item
                        if cleanName.hasPrefix("models--") {
                            cleanName = cleanName.replacingOccurrences(of: "models--", with: "").replacingOccurrences(of: "--", with: "/")
                        }

                        found.append(InstalledModelItem(
                            id: cleanName,
                            name: cleanName,
                            path: itemPath,
                            sizeBytes: size,
                            format: format,
                            sourceDir: base,
                            modifiedDate: mod
                        ))
                    }
                } else if item.hasSuffix(".gguf") || item.hasSuffix(".bin") {
                    let attrs = try? fm.attributesOfItem(atPath: itemPath)
                    let size = (attrs?[.size] as? Int64) ?? 0
                    let mod = (attrs?[.modificationDate] as? Date) ?? Date()
                    let format = item.hasSuffix(".gguf") ? "GGUF" : "Binary"
                    found.append(InstalledModelItem(
                        id: item,
                        name: item,
                        path: itemPath,
                        sizeBytes: size,
                        format: format,
                        sourceDir: base,
                        modifiedDate: mod
                    ))
                }
            }
        }

        return found
    }

    nonisolated private static func directorySize(at path: String) -> Int64 {
        let fm = FileManager.default
        guard let enumerator = fm.enumerator(atPath: path) else { return 0 }
        var total: Int64 = 0
        while let file = enumerator.nextObject() as? String {
            let full = (path as NSString).appendingPathComponent(file)
            if let attrs = try? fm.attributesOfItem(atPath: full),
               let s = attrs[.size] as? Int64 {
                total += s
            }
        }
        return total
    }

    public func revealInFinder(item: InstalledModelItem) {
        let url = URL(fileURLWithPath: item.path)
        NSWorkspace.shared.activateFileViewerSelecting([url])
    }

    public func deleteInstalledModel(item: InstalledModelItem) {
        try? FileManager.default.removeItem(atPath: item.path)
        scanInstalledModels()
    }

    public func onQueryChanged() {
        resultLimit = 30
        searchTask?.cancel()
        searchTask = Task {
            try? await Task.sleep(nanoseconds: 350_000_000) // 350ms debounce
            guard !Task.isCancelled else { return }
            await fetchTopModels()
        }
    }

    public func setFilter(_ filter: ModelFilter) {
        selectedFilter = filter
        resultLimit = 30
        searchTask?.cancel()
        searchTask = Task {
            guard !Task.isCancelled else { return }
            await fetchTopModels()
        }
    }

    /// LM Studio style pagination: grow the HF `limit` and refetch.
    public func loadMore() {
        guard !isLoading else { return }
        resultLimit = min(resultLimit + 30, 120)
        searchTask?.cancel()
        searchTask = Task { await fetchTopModels() }
    }

    public func fetchTopModels() async {
        fetchSeq += 1
        let mySeq = fetchSeq
        isLoading = true
        errorMessage = nil

        guard var urlComponents = URLComponents(string: "https://huggingface.co/api/models") else {
            isLoading = false
            errorMessage = "Could not build Hugging Face search URL. Showing last results."
            return
        }
        var queryItems: [URLQueryItem] = [
            URLQueryItem(name: "sort", value: "downloads"),
            URLQueryItem(name: "direction", value: "-1"),
            URLQueryItem(name: "limit", value: "\(resultLimit)")
        ]

        let query = searchQuery.trimmingCharacters(in: .whitespacesAndNewlines)
        if !query.isEmpty {
            queryItems.append(URLQueryItem(name: "search", value: query))
        } else {
            // Default top query for Apple Silicon: every MLX build,
            // not just one vendor family (Llama, DeepSeek, Gemma included).
            switch selectedFilter {
            case .appleSilicon:
                queryItems.append(URLQueryItem(name: "filter", value: "mlx"))
            case .mlx:
                queryItems.append(URLQueryItem(name: "filter", value: "mlx"))
            case .gguf:
                queryItems.append(URLQueryItem(name: "filter", value: "gguf"))
            case .coding:
                queryItems.append(URLQueryItem(name: "search", value: "coder"))
            case .reasoning:
                queryItems.append(URLQueryItem(name: "search", value: "reasoning"))
            case .vision:
                queryItems.append(URLQueryItem(name: "search", value: "vision"))
                queryItems.append(URLQueryItem(name: "filter", value: "mlx"))
            }
        }

        if !query.isEmpty {
            switch selectedFilter {
            case .appleSilicon, .mlx, .vision:
                queryItems.append(URLQueryItem(name: "filter", value: "mlx"))
            case .gguf:
                queryItems.append(URLQueryItem(name: "filter", value: "gguf"))
            default:
                break
            }
        }

        urlComponents.queryItems = queryItems

        guard let url = urlComponents.url else {
            isLoading = false
            errorMessage = "Could not encode Hugging Face search URL. Showing last results."
            return
        }

        var request = URLRequest(url: url)
        request.timeoutInterval = 10
        request.setValue("LAC-Studio/2.8", forHTTPHeaderField: "User-Agent")

        do {
            let (data, response) = try await URLSession.shared.data(for: request)
            // Superseded by a newer search: leave state to its owner.
            guard mySeq == fetchSeq else { return }
            guard let http = response as? HTTPURLResponse, http.statusCode == 200 else {
                let code = (response as? HTTPURLResponse)?.statusCode ?? -1
                self.errorMessage = "Hugging Face search failed (HTTP \(code)). Showing last results."
                self.isLoading = false
                return
            }

            // Per-element tolerance: one malformed HF entry must not fail
            // the whole page. Failable wrapper skips bad items.
            struct FailableModel: Decodable {
                let item: HFModelItem?
                init(from decoder: Decoder) throws {
                    item = try? HFModelItem(from: decoder)
                }
            }
            let raw = try JSONDecoder().decode([FailableModel].self, from: data)
            let decoded = raw.compactMap(\.item)
            if !Task.isCancelled {
                if !decoded.isEmpty {
                    self.models = decoded
                    self.hasMoreResults = decoded.count >= resultLimit && resultLimit < 120
                } else {
                    self.errorMessage = "No models matched this query on Hugging Face. Showing last results."
                }
                self.isLoading = false
            } else if mySeq == fetchSeq {
                // Cancelled but still latest (no replacement in flight):
                // release the spinner instead of sticking it.
                self.isLoading = false
            }
        } catch {
            // Superseded by a newer search: leave state to its owner.
            guard mySeq == fetchSeq else { return }
            self.isLoading = false
            if Task.isCancelled { return }
            if self.models.isEmpty {
                self.models = Self.curatedTopModels
                self.errorMessage = "Hugging Face unreachable — showing curated offline list."
            } else {
                self.errorMessage = "Hugging Face request failed (\(error.localizedDescription)). Showing last results."
            }
        }
    }

    // MARK: - Hugging Face Model File & Quantization Inspector (LM Studio style)

    public func inspectModelRepo(_ model: HFModelItem) {
        inspectSeq += 1
        let myInspect = inspectSeq
        inspectingModel = model
        repoFiles = []
        isLoadingRepoFiles = true
        repoFilesError = nil
        Task {
            await fetchRepoFiles(modelId: model.id, seq: myInspect)
        }
    }

    public func fetchRepoFiles(modelId: String, seq: Int? = nil) async {
        // Percent-encode each path segment (org / model names may contain
        // reserved characters) and fall back from `main` to `master`.
        let segments = modelId.split(separator: "/").map {
            $0.addingPercentEncoding(withAllowedCharacters: .urlPathAllowed) ?? String($0)
        }
        let encodedId = segments.joined(separator: "/")

        var lastError: String? = nil
        for branch in ["main", "master"] {
            guard let url = URL(string: "https://huggingface.co/api/models/\(encodedId)/tree/\(branch)") else {
                continue
            }

            var req = URLRequest(url: url)
            req.timeoutInterval = 10
            req.setValue("LAC-Studio/2.8", forHTTPHeaderField: "User-Agent")

            do {
                let (data, res) = try await URLSession.shared.data(for: req)
                guard let http = res as? HTTPURLResponse, http.statusCode == 200 else {
                    lastError = "Could not fetch file manifest for \(modelId) (branch \(branch): HTTP \((res as? HTTPURLResponse)?.statusCode ?? -1))"
                    continue
                }
                struct FailableFile: Decodable {
                    let item: HFRepoFileItem?
                    init(from decoder: Decoder) throws {
                        item = try? HFRepoFileItem(from: decoder)
                    }
                }
                let raw = try JSONDecoder().decode([FailableFile].self, from: data)
                let decoded = raw.compactMap(\.item)
                // Superseded by a newer inspect: leave state to its owner.
                if let seq, seq != inspectSeq { return }
                let filesOnly = decoded.filter {
                    $0.type != "directory" && ($0.isGguf || $0.isSafetensors || $0.path.hasSuffix(".json") || $0.path.hasSuffix(".bin"))
                }
                self.repoFiles = filesOnly.sorted { ($0.size ?? 0) > ($1.size ?? 0) }
                self.isLoadingRepoFiles = false
                self.repoFilesError = nil
                return
            } catch {
                if Task.isCancelled { return }
                lastError = error.localizedDescription
            }
        }
        self.isLoadingRepoFiles = false
        self.repoFilesError = lastError ?? "Could not fetch file manifest for \(modelId)"
    }
}
