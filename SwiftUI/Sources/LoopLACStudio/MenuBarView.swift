import AppKit
import SwiftUI

// MARK: - Menu bar extra: live gateway status and quick controls outside the window

struct MenuBarView: View {
    @EnvironmentObject private var network: NetworkManager

    var body: some View {
        if let r = network.response {
            Label("LAC Gateway :\(network.port) · Online", systemImage: "checkmark.circle.fill")
            Text("Active Engine: \(r.active.uppercased()) (Port \(r.target_port))")
                .font(.caption)
            Text("Uptime: \(formatUptime(r.uptime_secs)) · \(r.models_mapped) models mapped")
                .font(.caption)

            Divider()

            // Engine Switcher
            Menu("Switch Engine (Target: \(r.preferred))") {
                Button("Auto (Tier Priority: MLX → llama → Ollama)") {
                    Task { await network.switchBackend("auto") }
                }
                Button("MLX Native (Apple Silicon Unified Memory)") {
                    Task { await network.switchBackend("mlx") }
                }
                Button("llama.cpp (Metal FP16/INT8)") {
                    Task { await network.switchBackend("llama") }
                }
                Button("Ollama Resident") {
                    Task { await network.switchBackend("ollama") }
                }
                Button("Fastest (Lowest EWMA Latency)") {
                    Task { await network.switchBackend("fastest") }
                }
            }
        } else {
            Label("LAC Router Offline", systemImage: "exclamationmark.triangle.fill")
            Text("Port :\(network.port) not responding")
                .font(.caption)
            Button("Start Router Daemon") {
                Task { await network.startRouter() }
            }
        }

        // Host Telemetry
        if let h = network.host {
            Divider()
            if let therm = h.thermal {
                Label("Thermals: \(therm.capitalized)", systemImage: therm == "nominal" ? "thermometer.sun.fill" : "exclamationmark.shield.fill")
            }
            if let free = h.free_ram_gib, let total = h.total_ram_gib {
                Label(String(format: "RAM: %.1f / %.1f GiB Free", free, total), systemImage: "memorychip.fill")
            }
        }

        Divider()

        Button("Open Loop LAC Studio") {
            AppDelegate.showMainWindow()
        }
        .keyboardShortcut("o", modifiers: .command)

        Button("Refresh Telemetry") {
            network.fetch()
        }
        .keyboardShortcut("r", modifiers: .command)

        Divider()

        Button("Quit Loop LAC Studio") {
            NSApplication.shared.terminate(nil)
        }
        .keyboardShortcut("q", modifiers: .command)
    }

    private func formatUptime(_ secs: Int) -> String {
        if secs < 60 { return "\(secs)s" }
        let mins = secs / 60
        if mins < 60 { return "\(mins)m \(secs % 60)s" }
        let hrs = mins / 60
        return "\(hrs)h \(mins % 60)m"
    }
}
