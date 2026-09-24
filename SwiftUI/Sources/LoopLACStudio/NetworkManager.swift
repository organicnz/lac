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

/// Decodes every router generation: v2.7+ fields (uptime, inflight,
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
    @Published var pullProgress: Double?
    private var pullProcess: Process?

    /// Shared tailnet-ready connection (local 127.0.0.1 default, remote
    /// Tailscale IP/MagicDNS + Keychain Bearer token). All URLs flow here.
    var connection: LACConnectionStore { LACConnectionStore.shared }

    var port: Int { connection.port }
    var displayHost: String { connection.displayName }
    var isRemote: Bool { connection.isRemote }

    init() {
        checkDaemonStatus()
    }

    /// Resolve the `lac` binary: explicit env > ~/.local/bin > /opt/homebrew >
    /// /usr/local > PATH. Returns the executable URL plus an argument prefix:
    /// the PATH fallback goes through /usr/bin/env because a bare `"lac"`
    /// is not a valid file URL (`Process.run` would always throw).
    nonisolated static func lacExecutable() -> (url: URL, prefix: [String]) {
        let fm = FileManager.default
        if let env = ProcessInfo.processInfo.environment["LAC_BIN"], fm.isExecutableFile(atPath: env) {
            return (URL(fileURLWithPath: env), [])
        }
        let home = NSHomeDirectory()
        for cand in ["\(home)/.local/bin/lac", "/opt/homebrew/bin/lac", "/usr/local/bin/lac", "/usr/bin/lac"] {
            if fm.isExecutableFile(atPath: cand) { return (URL(fileURLWithPath: cand), []) }
        }
        return (URL(fileURLWithPath: "/usr/bin/env"), ["lac"])
    }

    func checkDaemonStatus() {
        let home = NSHomeDirectory()
        let plist = "\(home)/Library/LaunchAgents/org.lac.router.plist"
        daemonInstalled = FileManager.default.fileExists(atPath: plist)
    }

    func fetch(clearLastError: Bool = true) {
        // Coalesce overlapping polls (3s dashboard ticker + manual refresh).
        guard !isChecking else { return }
        isChecking = true
        Task { @MainActor in
            do {
                guard let url = connection.url(path: "/lac/status") else {
                    await MainActor.run {
                        self.lastError = "Invalid router URL (\(connection.displayName))"
                    }
                    return
                }
                var req = URLRequest(url: url)
                req.timeoutInterval = 5
                connection.authorize(&req)
                let (data, resp) = try await URLSession.shared.data(for: req)
                if let http = resp as? HTTPURLResponse, http.statusCode == 401 {
                    throw NSError(domain: "LAC", code: 401, userInfo: [NSLocalizedDescriptionKey: "401 Unauthorized — set the gateway token (Tailscale remote)."])
                }
                let decoded = try JSONDecoder().decode(RouterStatusResponse.self, from: data)
                await MainActor.run {
                    self.response = decoded
                    if clearLastError { self.lastError = nil }
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
    func runLac(_ args: [String], timeoutSeconds: Double = 30) async -> String {
        await withCheckedContinuation { cont in
            DispatchQueue.global().async {
                let exe = Self.lacExecutable()
                let p = Process()
                p.executableURL = exe.url
                p.arguments = exe.prefix + args
                let pipe = Pipe()
                p.standardOutput = pipe
                p.standardError = pipe
                // Resume-once guard: the timeout path (kill) and the wait
                // path below race by design, so the flag is lock-guarded —
                // a double-resume crashes a CheckedContinuation.
                let state = NSLock()
                var resumed = false
                func resumeOnce(_ s: String) {
                    state.lock()
                    defer { state.unlock() }
                    guard !resumed else { return }
                    resumed = true
                    cont.resume(returning: s)
                }
                // Timeout escalates TERM → INT → KILL. The `waitUntilExit`
                // below then returns and the real (partial) output is
                // delivered — a SIGTERM-ignoring child can no longer wedge
                // this worker thread forever.
                DispatchQueue.global().asyncAfter(deadline: .now() + timeoutSeconds) {
                    guard p.isRunning else { return }
                    p.terminate()
                    DispatchQueue.global().asyncAfter(deadline: .now() + 2) {
                        guard p.isRunning else { return }
                        p.interrupt()
                        DispatchQueue.global().asyncAfter(deadline: .now() + 2) {
                            if p.isRunning { Darwin.kill(p.processIdentifier, SIGKILL) }
                        }
                    }
                }
                do {
                    try p.run()
                    p.waitUntilExit()
                    let s = String(data: pipe.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8) ?? ""
                    resumeOnce(s)
                } catch {
                    resumeOnce("")
                }
            }
        }
    }

    func spawnDetachedLac(_ args: [String]) {
        DispatchQueue.global().async {
            let exe = Self.lacExecutable()
            let p = Process()
            p.executableURL = exe.url
            p.arguments = exe.prefix + args
            let nullDev = FileHandle.nullDevice
            p.standardOutput = nullDev
            p.standardError = nullDev
            try? p.run()
        }
    }

    func startMLX() async {
        guard !isRemote else { lastAction = "Remote tailnet: start backends on the Mac, not here."; return }
        spawnDetachedLac(["serve", "mlx"])
        for _ in 0..<10 {
            try? await Task.sleep(nanoseconds: 500_000_000)
            if await checkPortHealth(8080) { break }
        }
        fetch()
    }

    func startLlama() async {
        guard !isRemote else { lastAction = "Remote tailnet: start backends on the Mac, not here."; return }
        spawnDetachedLac(["serve", "llama"])
        for _ in 0..<10 {
            try? await Task.sleep(nanoseconds: 500_000_000)
            if await checkPortHealth(8081) { break }
        }
        fetch()
    }

    func startOllama() async {
        guard !isRemote else { lastAction = "Remote tailnet: start backends on the Mac, not here."; return }
        spawnDetachedLac(["serve", "ollama"])
        for _ in 0..<10 {
            try? await Task.sleep(nanoseconds: 500_000_000)
            if await checkPortHealth(11434) { break }
        }
        fetch()
    }

    func checkPortHealth(_ targetPort: Int) async -> Bool {
        if isRemote {
            let url = connection.url(path: "/v1/models")
            guard let url else { return false }
            var req = URLRequest(url: url)
            req.timeoutInterval = 3
            connection.authorize(&req)
            do {
                let (_, resp) = try await URLSession.shared.data(for: req)
                guard let http = resp as? HTTPURLResponse, http.statusCode == 200 else { return false }
                return true
            } catch {
                return false
            }
        }
        let url = URL(string: "http://127.0.0.1:\(targetPort)/v1/models")
        guard let url else { return false }
        var req = URLRequest(url: url)
        req.timeoutInterval = 1
        do {
            let (_, resp) = try await URLSession.shared.data(for: req)
            guard let http = resp as? HTTPURLResponse, http.statusCode == 200 else { return false }
            return true
        } catch {
            return false
        }
    }

    /// Free bytes on the home volume (for pull disk preflight).
    static func freeDiskBytes(path: String = NSHomeDirectory()) -> Int64 {
        let attrs = try? FileManager.default.attributesOfFileSystem(forPath: path)
        return (attrs?[.systemFreeSize] as? NSNumber)?.int64Value ?? 0
    }

    static func formatGB(_ bytes: Int64) -> String {
        let gb = Double(bytes) / (1024.0 * 1024.0 * 1024.0)
        if gb >= 1.0 { return String(format: "%.1f GB", gb) }
        return String(format: "%.0f MB", Double(bytes) / (1024.0 * 1024.0))
    }

    func stopAll() async { _ = await runLac(["stop"]) }

    func pullModel(_ modelId: String, expectedBytes: Int64? = nil) {
        guard pullingModelId == nil else { return }
        // Disk preflight: refuse before spawning when the download
        // provably does not fit (15% headroom for temp files).
        if let expected = expectedBytes, expected > 0 {
            let free = Self.freeDiskBytes()
            let need = Int64(Double(expected) * 1.15)
            if free > 0, free < need {
                let msg = "Pull blocked: \(modelId) needs ~\(Self.formatGB(need)) but only \(Self.formatGB(free)) is free. Inspect Files for a smaller quant or free disk space."
                pullOutput = msg
                lastError = msg
                lastAction = "Pull blocked (disk full)"
                return
            }
        }
        pullingModelId = modelId
        pullOutput = "Starting background download for \(modelId)..."
        pullProgress = nil
        lastAction = "Pulling \(modelId)..."
        
        // The Process is created and published synchronously on MainActor
        // BEFORE the background run starts: the old shape assigned
        // `pullProcess` via a queued main.async, so a fast Cancel in the
        // gap read nil and the child leaked. Only run/wait goes to .global.
        let exe = Self.lacExecutable()
        let p = Process()
        p.executableURL = exe.url
        p.arguments = exe.prefix + ["pull", modelId]
        let pipe = Pipe()
        p.standardOutput = pipe
        p.standardError = pipe

        let fileHandle = pipe.fileHandleForReading
        fileHandle.readabilityHandler = { [weak self] handle in
            guard let self else { return }
            let data = handle.availableData
            if !data.isEmpty, let str = String(data: data, encoding: .utf8) {
                DispatchQueue.main.async { [weak self, str] in
                    guard let self else { return }
                    let combined = (self.pullOutput ?? "") + str
                    // Keep only the last 2000 characters to avoid memory bloat
                    self.pullOutput = combined.count > 2000
                        ? String(combined.suffix(2000))
                        : combined
                    self.pullProgress = Self.parseProgress(from: combined)
                }
            }
        }

        self.pullProcess = p

        DispatchQueue.global().async {
            do {
                try p.run()
                p.waitUntilExit()
                fileHandle.readabilityHandler = nil

                DispatchQueue.main.async {
                    if p.terminationStatus != 0 {
                        // Terminated by user cancel (SIGTERM) — not a failure.
                        if p.terminationStatus == 15 {
                            self.lastAction = "Pull cancelled"
                        } else {
                            self.lastError = "Pull failed with status \(p.terminationStatus)"
                        }
                    } else {
                        self.pullProgress = 1.0
                    }
                    self.pullingModelId = nil
                    self.pullProcess = nil
                    self.lastAction = p.terminationStatus == 0 ? "Pulled \(modelId)" : (self.lastAction ?? "Pull ended")
                    self.fetch()
                }
            } catch {
                DispatchQueue.main.async {
                    self.lastError = error.localizedDescription
                    self.pullingModelId = nil
                    self.pullProcess = nil
                    self.pullProgress = nil
                    self.fetch()
                }
            }
        }
    }

    /// Cancel an in-flight `lac pull`. Terminates the child process;
    /// the completion handler records "cancelled", not a failure.
    func cancelPull() {
        pullProcess?.terminate()
        lastAction = "Cancelling pull..."
    }

    /// Best-effort parse of `NN%` progress from pull tool output.
    /// Returns 0...1, or nil when no percentage is present yet.
    nonisolated static func parseProgress(from text: String) -> Double? {
        // Scan trailing lines first — progress usually lives at the tail.
        let lines = text.split(separator: "\n").suffix(8)
        var best: Double?
        for line in lines {
            var idx = line.startIndex
            while idx < line.endIndex {
                guard let pct = line[idx...].firstIndex(of: "%") else { break }
                // Walk backwards over digits + decimal point.
                var start = pct
                while start > line.startIndex {
                    let prev = line.index(before: start)
                    let c = line[prev]
                    if c.isNumber || c == "." { start = prev } else { break }
                }
                if let v = Double(line[start..<pct]) {
                    best = min(max(v / 100.0, 0.0), 1.0)
                }
                idx = line.index(after: pct)
            }
        }
        return best
    }

    /// Switch the router's preferred backend via the existing
    /// /lac/switch endpoint (no new endpoints), then refresh.
    /// - Parameter clearLastError: if false, `fetch()` won't nil the
    ///   switch error so the user can see what went wrong.
    func switchBackend(_ target: String, clearLastError: Bool = true) async {
        guard let url = connection.url(path: "/lac/switch?target=\(target)") else {
            lastError = "Invalid router URL (\(connection.displayName)) — cannot switch to \(target)"
            return
        }
        var req = URLRequest(url: url)
        req.timeoutInterval = 8
        connection.authorize(&req)
        do {
            let (_, resp) = try await URLSession.shared.data(for: req)
            guard let http = resp as? HTTPURLResponse, http.statusCode == 200 else {
                if clearLastError { lastError = "Router switch failed (HTTP \((resp as? HTTPURLResponse)?.statusCode ?? -1))" }
                return
            }
            lastAction = "Switched router → \(target)"
        } catch {
            if clearLastError { lastError = "Router switch failed: \(error.localizedDescription)" }
        }
        fetch(clearLastError: false)
    }

    /// Install/uninstall the launchd daemons, record the first output
    /// line, then refresh from real status (never assume success).
    func setDaemon(enabled: Bool) async {
        guard !isRemote else { lastAction = "Remote tailnet: manage daemons on the Mac."; return }
        let out = await runLac(["daemon", enabled ? "install" : "uninstall"])
        lastAction = out.split(separator: "\n").first.map(String.init)
        checkDaemonStatus()
        fetch()
    }

    /// Start the gateway in the background, poll until responsive, then refresh.
    /// No-op on remote tailnet (the Mac owns the router there).
    func startRouter() async {
        guard !isRemote else { return }
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
                    self.isChecking = false
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
        guard let url = connection.url(path: "/lac/status") else { return false }
        var req = URLRequest(url: url)
        req.timeoutInterval = 2
        connection.authorize(&req)
        guard let (data, resp) = try? await URLSession.shared.data(for: req),
              let http = resp as? HTTPURLResponse, http.statusCode == 200,
              let _ = try? JSONDecoder().decode(RouterStatusResponse.self, from: data) else {
            return false
        }
        return true
    }
}
