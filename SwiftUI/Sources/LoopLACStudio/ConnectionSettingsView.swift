import SwiftUI

// MARK: - ConnectionSettingsView: local vs Tailscale remote gateway
//
// Local: 127.0.0.1:8000, no token unless the router is explicitly secured.
// Remote: MagicDNS (mac.tailXXX.ts.net), port 443,
// Bearer token from `LAC_API_TOKEN` on the Mac. Remote profiles require TLS
// through `tailscale serve --https`; arbitrary public hosts and plaintext
// remote prompts are rejected before a request is created.

struct ConnectionSettingsView: View {
    @ObservedObject var connection: LACConnectionStore
    @EnvironmentObject private var network: NetworkManager
    @State private var testResult: String?
    @State private var isTesting = false

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text("Gateway Connection").font(.headline)
                Spacer()
                HStack(spacing: 6) {
                    Circle()
                        .fill(connection.isRemote ? Color.purple : Color.green)
                        .frame(width: 7, height: 7)
                    Text(connection.isRemote ? "REMOTE" : "LOCAL")
                        .font(.system(size: 9, weight: .bold))
                        .padding(.horizontal, 6)
                        .padding(.vertical, 2)
                        .background(Capsule().fill((connection.isRemote ? Color.purple : Color.green).opacity(0.18)))
                        .foregroundColor(connection.isRemote ? .purple : .green)
                }
            }

            HStack(spacing: 8) {
                TextField("mac.tailXXX.ts.net or 127.0.0.1", text: $connection.host)
                    .textFieldStyle(.roundedBorder)
                    .font(.system(size: 12, design: .monospaced))
                    .frame(maxWidth: .infinity)
                TextField("Port", value: $connection.port, format: .number)
                    .textFieldStyle(.roundedBorder)
                    .font(.system(size: 12, design: .monospaced))
                    .frame(width: 80)
                Toggle("TLS", isOn: $connection.useTLS)
                    .toggleStyle(.checkbox)
                    .font(.caption)
                    .help("Required for remote profiles; use tailscale serve --https.")
            }

            SecureField("Bearer token (LAC_API_TOKEN on the Mac) — required for remote", text: $connection.token)
                .textFieldStyle(.roundedBorder)
                .font(.system(size: 12, design: .monospaced))

            HStack(spacing: 8) {
                Text(connection.displayName)
                    .font(.system(size: 11, design: .monospaced))
                    .foregroundColor(.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                Spacer()
                if isTesting {
                    ProgressView().scaleEffect(0.7)
                }
                Button("Test") { testConnection() }
                    .controlSize(.small)
                    .lacGlass()
                    .disabled(isTesting)
                Button("Local") {
                    connection.host = "127.0.0.1"
                    connection.port = 8000
                    connection.useTLS = false
                    network.fetch()
                }
                .controlSize(.small)
                .lacGlass()
            }

            if connection.isRemote && connection.token.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                Text("Remote requires TLS and a token. Paste LAC_API_TOKEN from the Mac.")
                    .font(.caption)
                    .foregroundColor(.orange)
            }
            if let r = testResult {
                Text(r).font(.caption).foregroundColor(.secondary).lineLimit(3)
            }
            Text("Local actions (serve, daemon) are disabled while remote — manage those on the Mac.")
                .font(.caption)
                .foregroundColor(.secondary)
        }
        .padding(14)
        .liquidGlassCard(cornerRadius: 14, hoverable: false)
    }

    private func testConnection() {
        isTesting = true
        testResult = nil
        Task {
            let ok = await network.checkRouterHealth()
            await MainActor.run {
                isTesting = false
                if ok {
                    testResult = "✓ Reachable: \(connection.displayName)"
                } else {
                    testResult = "✗ Unreachable: \(network.lastError ?? "check host, port, token, tailscale status")"
                }
                network.fetch()
            }
        }
    }
}
