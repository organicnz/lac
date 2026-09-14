import AppKit
import SwiftUI


// MARK: - Views

struct DashboardView: View {
    @EnvironmentObject private var network: NetworkManager
    @State private var showingAlert = false
    @State private var lastRefresh: Date?

    var body: some View {
        VStack(spacing: 0) {
            headerSection
            Divider()
            mainContent
        }
        .alert("LAC Dashboard", isPresented: $showingAlert) {
            Button("OK", role: .cancel) {}
        } message: {
            Text(network.lastError ?? "Unknown error")
        }
        .onAppear { network.fetch() }
        .onChange(of: network.isChecking) { checking in
            if !checking && network.lastError == nil && network.response != nil {
                lastRefresh = Date()
            }
        }
        .task {
            // Adaptive auto-refresh: polls every 3s when healthy, every 4s when
            // disconnected or recovering. Resumes automatically as soon as the
            // router comes back online without requiring manual intervention.
            while !Task.isCancelled {
                let interval: Duration = (network.lastError != nil) ? .seconds(4) : .seconds(3)
                try? await Task.sleep(for: interval)
                if Task.isCancelled { break }
                await MainActor.run {
                    if !network.isChecking && !network.autoRefreshPaused {
                        network.fetch()
                    }
                }
            }
        }
    }

    private var headerSection: some View {
        HStack {
            Circle()
                .fill(network.response != nil ? Color.green : Color.red)
                .frame(width: 8, height: 8)
            VStack(alignment: .leading, spacing: 4) {
                Text("Loop LAC Studio Ops")
                    .font(.system(.title, design: .rounded))
                    .fontWeight(.bold)
                Text(network.response?.router ?? "Local Agentic Coding")
                    .font(.subheadline)
                    .foregroundColor(.secondary)
                if let last = lastRefresh {
                    Text("Updated \(last, style: .relative)")
                        .font(.caption)
                        .foregroundColor(.secondary)
                }
            }
            Spacer()
            Button {
                network.autoRefreshPaused = false
                network.fetch()
            } label: {
                Image(systemName: "arrow.clockwise")
                    .font(.system(size: 14, weight: .medium))
                    .frame(width: 32, height: 32)
            }
            .disabled(network.isChecking)
            .keyboardShortcut("r", modifiers: .command)
            .lacGlass()
            if network.autoRefreshPaused || network.lastError != nil {
                Button("Resume") {
                    network.autoRefreshPaused = false
                    network.lastError = nil
                    network.fetch()
                }
                .controlSize(.small)
                .lacGlass()
            } else {
                Button("Pause") {
                    network.autoRefreshPaused = true
                }
                .controlSize(.small)
                .lacGlass()
            }
        }
        .padding(14)
        .background(.ultraThinMaterial)
        .overlay(Divider().opacity(0.35), alignment: .bottom)
    }

    private var mainContent: some View {
        Group {
            if network.response == nil && !network.isChecking {
                EmptyStateView(network: network)
            } else if network.response == nil && network.isChecking {
                VStack(spacing: 12) {
                    ProgressView()
                    Text("Connecting to router...")
                        .font(.callout)
                        .foregroundColor(.secondary)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                ScrollView(showsIndicators: false) {
                    VStack(spacing: 20) {
                        statusSummary
                        backendsGrid
                        statsOverview
                        controlPanel
                        daemonSection
                        if let err = network.lastError {
                            Text(err).font(.caption).foregroundColor(.red)
                        }
                    }
                    .padding(20)
                }
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private var statusSummary: some View {
        let r = network.response
        let activeValue: String = {
            guard let r, !r.active.isEmpty else { return "—" }
            return r.active
        }()
        return HStack(spacing: 16) {
            StatBox(title: "Active", value: activeValue,
                    icon: "brain.head.profile", color: .blue)
            StatBox(title: "RAM Free",
                    value: network.host?.free_ram_gib.map { String(format: "%.1f GiB", $0) } ?? "—",
                    icon: "memorychip", color: .green)
            StatBox(title: "Thermal",
                    value: network.host?.thermal?.capitalized ?? "—",
                    icon: "thermometer", color: thermalColor(network.host?.thermal ?? ""))
        }
    }

    private var backendsGrid: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack { Text("Backends").font(.headline); Spacer() }
            if let backends = network.response?.backends {
                HStack(spacing: 12) {
                    ForEach(["mlx", "llama", "ollama"], id: \.self) { key in
                        BackendTile(key: key, status: backends[key] ?? BackendDetail(port: 0, up: false))
                    }
                }
            } else {
                Text("Router offline — start with `lac route --daemon`.")
                    .font(.callout).foregroundColor(.secondary)
            }
        }
        .padding(14)
        .liquidGlassCard(cornerRadius: 14, hoverable: false)
    }

    private var statsOverview: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack { Text("Throughput").font(.headline); Spacer() }
            if let stats = network.response?.stats, !stats.isEmpty {
                ForEach(stats.keys.sorted(), id: \.self) { key in
                    if let s = stats[key] {
                        HStack {
                            Text(key).font(.system(size: 12, weight: .medium)); Spacer()
                            Text("\(s.ok) ok / \(s.err) err").font(.system(size: 12)); Spacer()
                            if let ewma = s.ewma_ms {
                                Text("EWMA: \(Int(ewma))ms").font(.system(size: 12))
                            }
                        }
                        .monospacedDigit()
                    }
                }
            } else {
                Text("No measured traffic yet.").font(.caption).foregroundColor(.secondary)
            }
        }
        .padding(14)
        .liquidGlassCard(cornerRadius: 14, hoverable: false)
    }

    private var controlPanel: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack { Text("Deploy").font(.headline); Spacer() }
            HStack(spacing: 10) {
                Button("MLX") { Task { await network.startMLX() } }
                    .lacGlass()
                Button("Llama") { Task { await network.startLlama() } }
                    .lacGlass()
                Button("Ollama") { Task { await network.startOllama() } }
                    .lacGlass()
                Button("Stop All") { Task { await network.stopAll() } }
                    .lacGlass()
            }
            HStack(spacing: 8) {
                Text("Backend:").font(.caption).foregroundColor(.secondary)
                ForEach(["auto", "mlx", "llama", "ollama"], id: \.self) { target in
                    Button(target) { Task { await network.switchBackend(target) } }
                        .controlSize(.small)
                        .lacGlass()
                        .disabled(network.response?.preferred == target)
                }
            }
        }
        .padding(14)
        .liquidGlassCard(cornerRadius: 14, hoverable: false)
    }

    private var daemonSection: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack { Text("Daemon").font(.headline); Spacer() }
            HStack(spacing: 12) {
                Image(systemName: "server.rack").foregroundColor(.blue)
                VStack(alignment: .leading, spacing: 2) {
                    Text(network.daemonInstalled ? "LaunchAgent Installed" : "LaunchAgent Not Installed")
                        .font(.system(size: 13, weight: .medium))
                    if let r = network.response {
                        Text(daemonStatus(r))
                            .font(.caption)
                            .foregroundColor(.secondary)
                    } else {
                        Text("router unreachable")
                            .font(.caption)
                            .foregroundColor(.red)
                    }
                }
                Spacer()
                Button("Install") { Task { await network.setDaemon(enabled: true) } }
                    .controlSize(.small)
                    .disabled(network.daemonInstalled)
                    .lacGlass()
                Button("Uninstall") { Task { await network.setDaemon(enabled: false) } }
                    .controlSize(.small)
                    .disabled(!network.daemonInstalled)
                    .lacGlass()
            }
            if let action = network.lastAction {
                Text(action).font(.caption).foregroundColor(.secondary).lineLimit(2)
            }
        }
        .padding(14)
        .liquidGlassCard(cornerRadius: 14, hoverable: false)
    }

    // Older gateways omit uptime/inflight: show only real knowledge.
    private func daemonStatus(_ r: RouterStatusResponse) -> String {
        var parts = ["Preferred: \(r.preferred)"]
        if r.uptime_secs > 0 { parts.append("uptime \(r.uptime_secs)s") }
        if r.inflight > 0 { parts.append("inflight \(r.inflight)") }
        return parts.joined(separator: " • ")
    }

    private func StatBox(title: String, value: String, icon: String, color: Color) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack { Image(systemName: icon).foregroundColor(color); Spacer() }
            Text(title.uppercased()).font(.caption2).foregroundColor(.secondary)
            Text(value).font(.system(size: 17, weight: .semibold)).monospacedDigit()
        }
        .padding(14)
        .frame(maxWidth: .infinity, minHeight: 60)
        .liquidGlassCard(cornerRadius: 12, hoverable: true, tint: color)
    }

    private func BackendTile(key: String, status: BackendDetail) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                Image(systemName: "server.rack")
                    .foregroundColor(key == "mlx" ? .blue : key == "llama" ? .purple : .red)
                Text(key.uppercased()).font(.system(size: 13, weight: .medium))
                Spacer()
            }
            HStack {
                Circle().fill(status.up ? Color.green : Color.red).frame(width: 8, height: 8)
                Text(status.up ? "online" : "offline")
                    .font(.system(size: 11)).foregroundColor(.secondary)
                Text(":\(status.port)").font(.system(size: 12)).foregroundColor(.secondary)
            }
        }
        .padding(10)
        .frame(maxWidth: .infinity)
        .liquidGlassCard(cornerRadius: 10, hoverable: true, tint: status.up ? .green : .red)
    }

    private func thermalColor(_ thermal: String) -> Color {
        switch thermal.lowercased() {
        case "critical": return .red
        case "serious": return .orange
        case "fair": return .yellow
        default: return .green
        }
    }
}

struct EmptyStateView: View {
    @ObservedObject var network: NetworkManager

    var body: some View {
        VStack {
            Spacer()
            VStack(spacing: 16) {
                Image(systemName: "wifi.slash")
                    .font(.system(size: 40))
                    .foregroundColor(.secondary)
                Text("Router unreachable")
                    .font(.title2).fontWeight(.semibold)
                Text("Start the gateway, then retry. Nothing is served until a backend is up.")
                    .font(.callout).foregroundColor(.secondary)
                    .multilineTextAlignment(.center)
                if let err = network.lastError {
                    Text(err).font(.caption).foregroundColor(.red).lineLimit(3)
                }
                HStack(spacing: 12) {
                    Button("Retry") {
                        network.autoRefreshPaused = false
                        network.lastError = nil
                        network.fetch()
                    }
                    .lacGlassProminent()

                    Button("Start router") { Task { await network.startRouter() } }
                        .lacGlass()
                }
                if let action = network.lastAction {
                    Text(action).font(.caption).foregroundColor(.secondary).lineLimit(2)
                }
            }
            .padding(32)
            .liquidGlassCard(cornerRadius: 18, hoverable: false)
            .frame(maxWidth: 480)
            Spacer()
        }
        .padding(24)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

#Preview {
    DashboardView()
        .environmentObject(NetworkManager())
}
