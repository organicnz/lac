import Foundation
import Security
import SwiftUI

// MARK: - LACConnectionStore: local + Tailscale remote gateway profiles
//
// Single source of truth for every gateway URL in Studio. Local default is
// `http://127.0.0.1:8000` (no token). Remote is a Tailscale IP or MagicDNS
// name (e.g. `100.64.0.5` or `mac.tailabc123.ts.net`) with a Bearer token.
//
// Transport is WireGuard-encrypted by Tailscale, so plain `http` inside the
// tailnet is correct. Flip `useTLS` on only when fronting via
// `tailscale serve --https`.
//
// Host/port/TLS persist in UserDefaults; the token lives in Keychain
// (service `org.lac.studio`, account `gateway-token`), never on disk.

@MainActor
final class LACConnectionStore: ObservableObject {
    static let shared = LACConnectionStore()

    @Published var host: String {
        didSet { UserDefaults.standard.set(host, forKey: Self.hostKey); touch() }
    }
    @Published var port: Int {
        didSet { UserDefaults.standard.set(port, forKey: Self.portKey); touch() }
    }
    @Published var useTLS: Bool {
        didSet { UserDefaults.standard.set(useTLS, forKey: Self.tlsKey); touch() }
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
    }

    var isRemote: Bool {
        let h = host.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        return h != "127.0.0.1" && h != "localhost" && h != "::1" && !h.isEmpty
    }

    var scheme: String { useTLS ? "https" : "http" }

    var displayName: String { "\(scheme)://\(host):\(port)" }

    func url(path: String) -> URL? {
        let p = path.hasPrefix("/") ? path : "/\(path)"
        return URL(string: "\(scheme)://\(host):\(port)\(p)")
    }

    /// Attach `Authorization: Bearer` when a token is set AND the target is
    /// remote. Local loopback stays token-less for frictionless dev; remote
    /// without a token is fail-closed at the router (401), surfaced in UI.
    func authorize(_ req: inout URLRequest) {
        let t = token.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !t.isEmpty, isRemote else { return }
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
