import AppKit
import SwiftUI

// MARK: - Data Models (match `lac-router /lac/status` exactly)

struct BackendDetail: Codable {
    let port: Int
    let up: Bool
}

struct BackendStats: Codable {
    let port: Int?
    let ewma_ms: Double?
    let ok: UInt64
    let err: UInt64
    let est_tokens: UInt64?

    enum CodingKeys: String, CodingKey {
        case port, ewma_ms, ok, err, est_tokens
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        port = try c.decodeIfPresent(Int.self, forKey: .port)
        ewma_ms = try c.decodeIfPresent(Double.self, forKey: .ewma_ms)
        ok = (try? c.decodeIfPresent(UInt64.self, forKey: .ok)) ?? 0
        err = (try? c.decodeIfPresent(UInt64.self, forKey: .err)) ?? 0
        est_tokens = try c.decodeIfPresent(UInt64.self, forKey: .est_tokens)
    }
}

/// Decodes every router generation: v2.7 fields (uptime, inflight,
/// models_mapped, usage_log, stats) default when absent, so an older
/// gateway (e.g. v2.0) still renders instead of blanking the dashboard.
struct RouterStatusResponse: Codable {
    let status: String
    let router: String
    let preferred: String
    let active: String
    let target_port: Int
    let uptime_secs: Int
    let inflight: Int
    let models_mapped: Int
    let usage_log: String
    let backends: [String: BackendDetail]
    let stats: [String: BackendStats]

    enum CodingKeys: String, CodingKey {
        case status, router, preferred, active, target_port
        case uptime_secs, inflight, models_mapped, usage_log, backends, stats
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        status = try c.decode(String.self, forKey: .status)
        router = try c.decode(String.self, forKey: .router)
        preferred = try c.decode(String.self, forKey: .preferred)
        active = (try? c.decodeIfPresent(String.self, forKey: .active)) ?? "—"
        target_port = (try? c.decodeIfPresent(Int.self, forKey: .target_port)) ?? 0
        uptime_secs = (try? c.decodeIfPresent(Int.self, forKey: .uptime_secs)) ?? 0
        inflight = (try? c.decodeIfPresent(Int.self, forKey: .inflight)) ?? 0
        models_mapped = (try? c.decodeIfPresent(Int.self, forKey: .models_mapped)) ?? 0
        usage_log = (try? c.decodeIfPresent(String.self, forKey: .usage_log)) ?? ""
        backends = (try? c.decodeIfPresent([String: BackendDetail].self, forKey: .backends)) ?? [:]
        stats = (try? c.decodeIfPresent([String: BackendStats].self, forKey: .stats)) ?? [:]
    }
}

/// Best-effort host facts from `lac status --json` (separate endpoint,
/// keys may be absent on older builds — everything optional).
struct LacStatusJson: Codable {
    let model: String?
    let thermal: String?
    let free_ram_gib: Double?
    let total_ram_gib: Double?
    let gateway: Bool?
    let active: String?
}

// MARK: - Network Manager

@MainActor
class NetworkManager: ObservableObject {
    @Published var response: RouterStatusResponse?
    @Published var host: LacStatusJson?
    @Published var isChecking = false
    @Published var lastError: String?
    @Published var autoRefreshPaused = false
    @Published var lastAction: String?
    @Published var daemonInstalled = false
    @Published var hasAutoStartedRouter = false
    @Published var pullingModelId: String?
    @Published var pullOutput: String?

    let port: Int = {
        if let raw = ProcessInfo.processInfo.environment["LAC_ROUTER_PORT"],
           let p = Int(raw), p > 0 { return p }
        return 8000
    }()

    init() {
        checkDaemonStatus()
    }

    /// Resolve the `lac` binary: explicit env > ~/.local/bin > /opt/homebrew > /usr/local > PATH.
    nonisolated static func lacBinary() -> String {
        let fm = FileManager.default
        if let env = ProcessInfo.processInfo.environment["LAC_BIN"], fm.isExecutableFile(atPath: env) {
            return env
        }
        let home = NSHomeDirectory()
        for cand in ["\(home)/.local/bin/lac", "/opt/homebrew/bin/lac", "/usr/local/bin/lac", "/usr/bin/lac"] {
            if fm.isExecutableFile(atPath: cand) { return cand }
        }
        return "lac"
    }

    func checkDaemonStatus() {
        let home = NSHomeDirectory()
        let plist = "\(home)/Library/LaunchAgents/org.lac.router.plist"
        daemonInstalled = FileManager.default.fileExists(atPath: plist)
    }

    func fetch() {
        isChecking = true
        Task {
            do {
                let url = URL(string: "http://127.0.0.1:\(port)/lac/status")!
                var req = URLRequest(url: url)
                req.timeoutInterval = 5
                let (data, _) = try await URLSession.shared.data(for: req)
                let decoded = try JSONDecoder().decode(RouterStatusResponse.self, from: data)
                await MainActor.run {
                    self.response = decoded
                    self.lastError = nil
                    self.isChecking = false
                    self.autoRefreshPaused = false
                }
            } catch {
                await MainActor.run {
                    self.lastError = error.localizedDescription
                    self.isChecking = false
                    if !self.hasAutoStartedRouter {
                        self.hasAutoStartedRouter = true
                        Task { await self.startRouter() }
                    }
                    // Keep autoRefresh running so self-healing loop can reconnect when online
                }
            }
            await fetchHostFacts()
            await MainActor.run {
                self.checkDaemonStatus()
            }
        }
    }

    /// Host facts via `lac status --json` (never blocks the router card).
    func fetchHostFacts() async {
        let out = await runLac(["status", "--json"])
        guard let data = out.data(using: .utf8),
              let decoded = try? JSONDecoder().decode(LacStatusJson.self, from: data)
        else { return }
        await MainActor.run { self.host = decoded }
    }

    @discardableResult
    func runLac(_ args: [String]) async -> String {
        await withCheckedContinuation { cont in
            DispatchQueue.global().async {
                let p = Process()
                p.executableURL = URL(fileURLWithPath: Self.lacBinary())
                p.arguments = args
                let pipe = Pipe()
                p.standardOutput = pipe
                p.standardError = pipe
                do {
                    try p.run()
                    p.waitUntilExit()
                    let s = String(data: pipe.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8) ?? ""
                    cont.resume(returning: s)
                } catch {
                    cont.resume(returning: "")
                }
            }
        }
    }

    func spawnDetachedLac(_ args: [String]) {
        DispatchQueue.global().async {
            let p = Process()
            p.executableURL = URL(fileURLWithPath: Self.lacBinary())
            p.arguments = args
            let nullDev = FileHandle.nullDevice
            p.standardOutput = nullDev
            p.standardError = nullDev
            try? p.run()
        }
    }

    func startMLX() async {
        spawnDetachedLac(["serve", "mlx"])
        for _ in 0..<10 {
            try? await Task.sleep(nanoseconds: 500_000_000)
            if await checkPortHealth(8080) { break }
        }
        fetch()
    }

    func startLlama() async {
        spawnDetachedLac(["serve", "llama"])
        for _ in 0..<10 {
            try? await Task.sleep(nanoseconds: 500_000_000)
            if await checkPortHealth(8081) { break }
        }
        fetch()
    }

    func startOllama() async {
        spawnDetachedLac(["serve", "ollama"])
        for _ in 0..<10 {
            try? await Task.sleep(nanoseconds: 500_000_000)
            if await checkPortHealth(11434) { break }
        }
        fetch()
    }

    func checkPortHealth(_ targetPort: Int) async -> Bool {
        guard let url = URL(string: "http://127.0.0.1:\(targetPort)/v1/models") else { return false }
        var req = URLRequest(url: url)
        req.timeoutInterval = 1
        return (try? await URLSession.shared.data(for: req)) != nil
    }

    func stopAll() async { _ = await runLac(["stop"]) }

    func pullModel(_ modelId: String) {
        guard pullingModelId == nil else { return }
        pullingModelId = modelId
        pullOutput = "Starting background download for \(modelId)..."
        lastAction = "Pulling \(modelId)..."
        
        DispatchQueue.global().async {
            let p = Process()
            p.executableURL = URL(fileURLWithPath: Self.lacBinary())
            p.arguments = ["pull", modelId]
            let pipe = Pipe()
            p.standardOutput = pipe
            p.standardError = pipe
            
            let fileHandle = pipe.fileHandleForReading
            fileHandle.readabilityHandler = { handle in
                let data = handle.availableData
                if !data.isEmpty, let str = String(data: data, encoding: .utf8) {
                    DispatchQueue.main.async {
                        self.pullOutput = (self.pullOutput ?? "") + str
                        // Keep only the last 1000 characters to avoid memory bloat
                        if self.pullOutput!.count > 1000 {
                            self.pullOutput = String(self.pullOutput!.suffix(1000))
                        }
                    }
                }
            }
            
            do {
                try p.run()
                p.waitUntilExit()
                fileHandle.readabilityHandler = nil
                
                DispatchQueue.main.async {
                    if p.terminationStatus != 0 {
                        self.lastError = "Pull failed with status \(p.terminationStatus)"
                    }
                    self.pullingModelId = nil
                    self.lastAction = "Pulled \(modelId)"
                    self.fetch()
                }
            } catch {
                DispatchQueue.main.async {
                    self.lastError = error.localizedDescription
                    self.pullingModelId = nil
                    self.fetch()
                }
            }
        }
    }

    /// Switch the router's preferred backend via the existing
    /// /lac/switch endpoint (no new endpoints), then refresh.
    func switchBackend(_ target: String) async {
        if let url = URL(string: "http://127.0.0.1:\(port)/lac/switch?target=\(target)") {
            var req = URLRequest(url: url)
            req.timeoutInterval = 8
            _ = try? await URLSession.shared.data(for: req)
        }
        lastAction = "Switched router → \(target)"
        fetch()
    }

    /// Install/uninstall the launchd daemons, record the first output
    /// line, then refresh from real status (never assume success).
    func setDaemon(enabled: Bool) async {
        let out = await runLac(["daemon", enabled ? "install" : "uninstall"])
        lastAction = out.split(separator: "\n").first.map(String.init)
        checkDaemonStatus()
        fetch()
    }

    /// Start the gateway in the background, poll until responsive, then refresh.
    func startRouter() async {
        await MainActor.run {
            self.lastAction = "Starting lac-router on :\(port)..."
            self.isChecking = true
        }

        let home = NSHomeDirectory()
        // 1. If daemon plist exists, install / start via launchctl
        let plist = "\(home)/Library/LaunchAgents/org.lac.router.plist"
        if FileManager.default.fileExists(atPath: plist) {
            _ = await runLac(["daemon", "install"])
        } else {
            // 2. Otherwise start via `lac route --daemon`
            let out = await runLac(["route", "--daemon"])
            if !out.isEmpty {
                await MainActor.run {
                    self.lastAction = out.split(separator: "\n").first.map(String.init)
                }
            }
        }

        // 3. Fallback: if port is still not responding, execute lac-router binary directly
        let routerCandidates = [
            "\(home)/.local/bin/lac-router",
            "/opt/homebrew/bin/lac-router",
            "/usr/local/bin/lac-router"
        ]
        for cand in routerCandidates {
            if FileManager.default.isExecutableFile(atPath: cand) {
                let p = Process()
                p.executableURL = URL(fileURLWithPath: cand)
                p.arguments = ["\(port)"]
                p.standardOutput = FileHandle.nullDevice
                p.standardError = FileHandle.nullDevice
                try? p.run()
                break
            }
        }

        // Poll up to 6 times (3 seconds) for the router to become ready
        for _ in 0..<6 {
            try? await Task.sleep(nanoseconds: 500_000_000)
            if await checkRouterHealth() {
                await MainActor.run {
                    self.autoRefreshPaused = false
                    self.lastError = nil
                    self.fetch()
                }
                return
            }
        }

        await MainActor.run {
            self.fetch()
        }
    }

    func checkRouterHealth() async -> Bool {
        guard let url = URL(string: "http://127.0.0.1:\(port)/lac/status") else { return false }
        var req = URLRequest(url: url)
        req.timeoutInterval = 2
        guard let (data, resp) = try? await URLSession.shared.data(for: req),
              let http = resp as? HTTPURLResponse, http.statusCode == 200,
              let _ = try? JSONDecoder().decode(RouterStatusResponse.self, from: data) else {
            return false
        }
        return true
    }
}
