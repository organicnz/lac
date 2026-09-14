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
        if let tags = tags {
            for tag in tags {
                let lower = tag.lowercased()
                if lower.contains("4-bit") || lower.contains("4bit") || lower.contains("q4") { return "4-bit" }
                if lower.contains("8-bit") || lower.contains("8bit") || lower.contains("q8") { return "8-bit" }
                if lower.contains("3-bit") || lower.contains("3bit") || lower.contains("q3") { return "3-bit" }
                if lower.contains("fp16") { return "FP16" }
            }
        }
        if id.contains("4bit") || id.contains("4-bit") || id.contains("Q4") { return "4-bit" }
        if id.contains("8bit") || id.contains("8-bit") || id.contains("Q8") { return "8-bit" }
        return "MLX"
    }

    public var isMlx: Bool {
        tags?.contains("mlx") == true || id.lowercased().contains("mlx")
    }

    public var isGguf: Bool {
        tags?.contains("gguf") == true || id.lowercased().contains("gguf")
    }

    public var ramFitEstimate: (label: String, color: Color) {
        let lower = id.lowercased()
        if lower.contains("70b") || lower.contains("72b") {
            return ("64GB+ RAM Recommended", .red)
        } else if lower.contains("32b") || lower.contains("27b") {
            return ("16GB – 32GB RAM (Fits)", .green)
        } else if lower.contains("14b") || lower.contains("8b") || lower.contains("7b") || lower.contains("3b") || lower.contains("1.5b") {
            return ("8GB+ RAM (Lightweight)", .green)
        }
        return ("Apple Silicon Compatible", .blue)
    }
}

// MARK: - Hugging Face Repo File Item (LM Studio file inspection & quantization picker)

public struct HFRepoFileItem: Identifiable, Codable, Hashable {
    public var id: String { path }
    public let path: String
    public let size: Int64?
    public let type: String?

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
        case small = "≤ 8B (Fast)"
        case medium = "14B – 32B (Balanced)"
        case large = "70B+ (Heavy)"

        public var id: String { rawValue }

        public func matches(item: HFModelItem) -> Bool {
            let lower = item.id.lowercased()
            switch self {
            case .all:
                return true
            case .small:
                return lower.contains("0.5b") || lower.contains("1.5b") || lower.contains("3b") || lower.contains("7b") || lower.contains("8b") || lower.contains("-8b") || lower.contains("lite")
            case .medium:
                return lower.contains("14b") || lower.contains("27b") || lower.contains("32b") || lower.contains("30b")
            case .large:
                return lower.contains("70b") || lower.contains("72b") || lower.contains("120b") || lower.contains("405b")
            }
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

    private var searchTask: Task<Void, Never>?

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
        Task {
            await fetchTopModels()
            scanInstalledModels()
        }
    }

    // MARK: - On-Device Model Discovery (LM Studio style)

    public func scanInstalledModels() {
        isScanningInstalled = true
        var found: [InstalledModelItem] = []
        let fm = FileManager.default
        let home = NSHomeDirectory()

        // Scan free disk space on local APFS volume
        if let values = try? URL(fileURLWithPath: home).resourceValues(forKeys: [.volumeAvailableCapacityForImportantUsageKey, .volumeAvailableCapacityKey]) {
            freeDiskBytes = values.volumeAvailableCapacityForImportantUsage ?? Int64(values.volumeAvailableCapacity ?? 0)
        }

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

        found.sort { $0.sizeBytes > $1.sizeBytes }
        installedModels = found
        totalInstalledBytes = found.reduce(0) { $0 + $1.sizeBytes }
        isScanningInstalled = false
    }

    private func directorySize(at path: String) -> Int64 {
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
        searchTask?.cancel()
        searchTask = Task {
            try? await Task.sleep(nanoseconds: 350_000_000) // 350ms debounce
            guard !Task.isCancelled else { return }
            await fetchTopModels()
        }
    }

    public func setFilter(_ filter: ModelFilter) {
        selectedFilter = filter
        searchTask?.cancel()
        searchTask = Task {
            guard !Task.isCancelled else { return }
            await fetchTopModels()
        }
    }

    public func fetchTopModels() async {
        isLoading = true
        errorMessage = nil

        guard var urlComponents = URLComponents(string: "https://huggingface.co/api/models") else {
            isLoading = false
            return
        }
        var queryItems: [URLQueryItem] = [
            URLQueryItem(name: "sort", value: "downloads"),
            URLQueryItem(name: "direction", value: "-1"),
            URLQueryItem(name: "limit", value: "30")
        ]

        let query = searchQuery.trimmingCharacters(in: .whitespacesAndNewlines)
        if !query.isEmpty {
            queryItems.append(URLQueryItem(name: "search", value: query))
        } else {
            // Default top query for Apple Silicon
            switch selectedFilter {
            case .appleSilicon:
                queryItems.append(URLQueryItem(name: "search", value: "qwen"))
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
            return
        }

        var request = URLRequest(url: url)
        request.timeoutInterval = 10
        request.setValue("LAC-Studio/2.7", forHTTPHeaderField: "User-Agent")

        do {
            let (data, response) = try await URLSession.shared.data(for: request)
            guard let http = response as? HTTPURLResponse, http.statusCode == 200 else {
                isLoading = false
                return
            }

            let decoded = try JSONDecoder().decode([HFModelItem].self, from: data)
            if !Task.isCancelled {
                if !decoded.isEmpty {
                    self.models = decoded
                }
                self.isLoading = false
            }
        } catch {
            if !Task.isCancelled {
                self.isLoading = false
                if self.models.isEmpty {
                    self.models = Self.curatedTopModels
                }
            }
        }
    }

    // MARK: - Hugging Face Model File & Quantization Inspector (LM Studio style)

    public func inspectModelRepo(_ model: HFModelItem) {
        inspectingModel = model
        repoFiles = []
        isLoadingRepoFiles = true
        repoFilesError = nil
        Task {
            await fetchRepoFiles(modelId: model.id)
        }
    }

    public func fetchRepoFiles(modelId: String) async {
        guard let url = URL(string: "https://huggingface.co/api/models/\(modelId)/tree/main") else {
            isLoadingRepoFiles = false
            return
        }

        var req = URLRequest(url: url)
        req.timeoutInterval = 10
        req.setValue("LAC-Studio/2.7", forHTTPHeaderField: "User-Agent")

        do {
            let (data, res) = try await URLSession.shared.data(for: req)
            guard let http = res as? HTTPURLResponse, http.statusCode == 200 else {
                isLoadingRepoFiles = false
                repoFilesError = "Could not fetch file manifest for \(modelId)"
                return
            }
            let decoded = try JSONDecoder().decode([HFRepoFileItem].self, from: data)
            let filesOnly = decoded.filter {
                $0.type != "directory" && ($0.isGguf || $0.isSafetensors || $0.path.hasSuffix(".json") || $0.path.hasSuffix(".bin"))
            }
            self.repoFiles = filesOnly.sorted { ($0.size ?? 0) > ($1.size ?? 0) }
            self.isLoadingRepoFiles = false
        } catch {
            if !Task.isCancelled {
                self.isLoadingRepoFiles = false
                self.repoFilesError = error.localizedDescription
            }
        }
    }
}
