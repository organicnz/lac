import Foundation
import Security
import SwiftUI

// MARK: - LACConnectionStore: local + Tailscale remote gateway profiles
//
// Single source of truth for every gateway URL in Studio. Local default is
// `http://127.0.0.1:8000` (no token). Remote is a Tailscale MagicDNS name
// (e.g. `mac.tailabc123.ts.net`) with TLS and a Bearer token.
//
// Remote profiles are restricted to Tailscale addresses and require TLS via
// `tailscale serve --https`; this prevents a typo or public DNS endpoint from
// receiving prompts or the gateway token. Local loopback HTTP remains open.
//
// Host/port/TLS persist in UserDefaults; the token lives in Keychain
// (service `org.lac.studio`, account `gateway-token`), never on disk.

@MainActor
final class LACConnectionStore: ObservableObject {
    static let shared = LACConnectionStore()

    @Published var host: String {
        didSet {
            let next = Self.normalized(host)
            if host != next { host = next }
            if Self.normalized(oldValue) != next {
                if !token.isEmpty { token = "" }
                if Self.isMagicDNSHost(next) {
                    useTLS = true
                } else if Self.isLoopback(next) {
                    useTLS = false
                }
            }
            UserDefaults.standard.set(host, forKey: Self.hostKey)
            touch()
        }
    }
    @Published var port: Int {
        didSet {
            if oldValue != port { token = "" }
            UserDefaults.standard.set(port, forKey: Self.portKey)
            touch()
        }
    }
    @Published var useTLS: Bool {
        didSet {
            if oldValue != useTLS { token = "" }
            UserDefaults.standard.set(useTLS, forKey: Self.tlsKey)
            touch()
        }
    }
    /// In-memory token; persisted to Keychain on set.
    @Published var token: String {
        didSet { Self.saveToken(token) }
    }
    /// Bump to force URLSession clients to re-resolve (host switch).
    @Published private(set) var revision: UInt64 = 0

    private static let hostKey = "lac.gateway.host"
    private static let portKey = "lac.gateway.port"
    private static let tlsKey = "lac.gateway.tls"

    init(
        host: String? = nil,
        port: Int? = nil,
        useTLS: Bool? = nil,
        token: String? = nil
    ) {
        let env = ProcessInfo.processInfo.environment
        let d = UserDefaults.standard
        let defaultHost = env["LAC_ROUTER_HOST"]?.trimmingCharacters(in: .whitespacesAndNewlines)
        let defaultPort = env["LAC_ROUTER_PORT"].flatMap { Int($0) }
        self.host = host
            ?? (defaultHost?.isEmpty == false ? defaultHost! : d.string(forKey: Self.hostKey))
            ?? "127.0.0.1"
        let storedPort = d.object(forKey: Self.portKey) as? Int
        self.port = port ?? defaultPort ?? (storedPort ?? 0 > 0 ? storedPort! : 8000)
        self.useTLS = useTLS ?? d.object(forKey: Self.tlsKey) as? Bool ?? false
        self.token = token ?? Self.loadToken() ?? env["LAC_API_TOKEN"] ?? ""
        // Normalize empty host once (fresh installs, cleared defaults).
        if self.host.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            self.host = "127.0.0.1"
        }
        if isRemote && useTLS == nil {
            self.useTLS = true
        }
    }

    private static func normalized(_ value: String) -> String {
        var h = value.trimmingCharacters(in: .whitespacesAndNewlines)
        if h.count >= 2, h.hasPrefix("["), h.hasSuffix("]") {
            h.removeFirst()
            h.removeLast()
        }
        return h
    }

    private static func isLoopback(_ value: String) -> Bool {
        let h = value.lowercased()
        if h == "localhost" || h == "::1" { return true }
        let parts = h.split(separator: ".")
        if parts.count == 4, let first = Int(parts[0]), first == 127,
           parts.dropFirst().allSatisfy({ Int($0).map { (0...255).contains($0) } == true }) {
            return true
        }
        return false
    }

    private static func isMagicDNSHost(_ value: String) -> Bool {
        value.lowercased().hasSuffix(".ts.net")
    }

    var normalizedHost: String { Self.normalized(host) }

    var isRemote: Bool { !Self.isLoopback(normalizedHost) }

    var scheme: String { useTLS ? "https" : "http" }

    var displayName: String { "\(scheme)://\(normalizedHost):\(port)" }

    func url(path: String) -> URL? {
        let h = normalizedHost
        guard !h.isEmpty,
              (1...65535).contains(port),
              !h.contains(where: { $0.isWhitespace || "/?#@".contains($0) }),
              !isRemote || (useTLS && Self.isMagicDNSHost(h)) else {
            return nil
        }
        let p = path.hasPrefix("/") ? path : "/\(path)"
        var components = URLComponents()
        components.scheme = scheme
        components.host = h
        components.port = port
        if let queryStart = p.firstIndex(of: "?") {
            components.path = String(p[..<queryStart])
            components.query = String(p[p.index(after: queryStart)...])
        } else {
            components.path = p
        }
        return components.url
    }

    /// Attach `Authorization: Bearer` whenever a token is configured. A
    /// router with a token requires it on loopback too, which prevents local
    /// reverse proxies from bypassing the remote gate.
    func authorize(_ req: inout URLRequest) {
        let t = token.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !t.isEmpty, !t.contains(where: { $0.isNewline }) else { return }
        req.setValue("Bearer \(t)", forHTTPHeaderField: "Authorization")
    }

    func clearToken() { token = "" }

    private func touch() { revision &+= 1 }

    // MARK: Keychain (token only)

    private static let service = "org.lac.studio"
    private static let account = "gateway-token"

    static func loadToken() -> String? {
        let q: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        var item: CFTypeRef?
        guard SecItemCopyMatching(q as CFDictionary, &item) == errSecSuccess,
              let data = item as? Data,
              let s = String(data: data, encoding: .utf8),
              !s.isEmpty
        else { return nil }
        return s
    }

    @discardableResult
    static func saveToken(_ token: String) -> Bool {
        let data = Data(token.utf8)
        let q: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
        if token.isEmpty {
            SecItemDelete(q as CFDictionary)
            return true
        }
        let attrs: [String: Any] = [kSecValueData as String: data]
        let st = SecItemUpdate(q as CFDictionary, attrs as CFDictionary)
        if st == errSecSuccess { return true }
        if st == errSecItemNotFound {
            var add = q
            add[kSecValueData as String] = data
            return SecItemAdd(add as CFDictionary, nil) == errSecSuccess
        }
        return false
    }
}
