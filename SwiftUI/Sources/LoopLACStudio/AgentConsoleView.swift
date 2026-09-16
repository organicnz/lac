import AppKit
import Foundation
import SwiftUI

// MARK: - Agent Console Store

@MainActor
public final class AgentConsoleStore: ObservableObject {
    @Published public var output: String = ""
    @Published public var isRunning: Bool = false
    @Published public var lastExitCode: Int32?
    @Published public var activeCommand: String?
    @Published public var executionDuration: TimeInterval = 0

    private var activeProcess: Process?
    private var timer: Timer?
    private var startTime: Date?

    public init() {}

    // MARK: Built-in Command Runners

    public func runCargoTests(repoRoot: URL) {
        let cargoManifest = repoRoot.appendingPathComponent("rust-src/Cargo.toml").path
        execute(
            command: "/usr/bin/env",
            arguments: ["cargo", "test", "--manifest-path", cargoManifest],
            workingDirectory: repoRoot,
            title: "cargo test (Rust Backend)"
        )
    }

    public func runSwiftTests(repoRoot: URL) {
        let swiftDir = repoRoot.appendingPathComponent("SwiftUI")
        execute(
            command: "/usr/bin/swift",
            arguments: ["test"],
            workingDirectory: swiftDir,
            title: "swift test (Loop LAC Studio)"
        )
    }

    public func runGitDiff(repoRoot: URL) {
        execute(
            command: "/usr/bin/git",
            arguments: ["diff"],
            workingDirectory: repoRoot,
            title: "git diff (Working Tree Audit)"
        )
    }

    public func runStackHealth(repoRoot: URL) {
        let candidates = [
            repoRoot.appendingPathComponent("rust-src/target/debug/lac"),
            repoRoot.appendingPathComponent("target/debug/lac"),
            URL(fileURLWithPath: "\(NSHomeDirectory())/.local/bin/lac"),
            URL(fileURLWithPath: "/opt/homebrew/bin/lac"),
            URL(fileURLWithPath: "/usr/local/bin/lac")
        ]
        if let found = candidates.first(where: { FileManager.default.isExecutableFile(atPath: $0.path) }) {
            execute(
                command: found.path,
                arguments: ["status"],
                workingDirectory: repoRoot,
                title: "lac status"
            )
        } else {
            execute(
                command: "/usr/bin/env",
                arguments: ["cargo", "run", "--manifest-path", repoRoot.appendingPathComponent("rust-src/Cargo.toml").path, "--bin", "lac", "--", "status"],
                workingDirectory: repoRoot,
                title: "lac status (cargo run)"
            )
        }
    }

    public func execute(command: String, arguments: [String], workingDirectory: URL, title: String) {
        guard !isRunning else { return }

        self.isRunning = true
        self.lastExitCode = nil
        self.activeCommand = title
        self.executionDuration = 0
        self.output = "⚡ Running: \(title)\nDirectory: \(workingDirectory.path)\n\n"
        let start = Date()
        self.startTime = start

        timer?.invalidate()
        timer = Timer.scheduledTimer(withTimeInterval: 0.1, repeats: true) { [weak self] _ in
            let duration = Date().timeIntervalSince(start)
            Task { @MainActor [weak self] in
                self?.executionDuration = duration
            }
        }

        let proc = Process()
        proc.executableURL = URL(fileURLWithPath: command)
        proc.arguments = arguments
        proc.currentDirectoryURL = workingDirectory

        var env = ProcessInfo.processInfo.environment
        env["PATH"] = (env["PATH"] ?? "") + ":/usr/local/bin:/opt/homebrew/bin:~/.cargo/bin"
        proc.environment = env

        let outPipe = Pipe()
        let errPipe = Pipe()
        proc.standardOutput = outPipe
        proc.standardError = errPipe

        self.activeProcess = proc

        // 30s timeout: TERM → after 2s INT → after 2s KILL.
        DispatchQueue.global().asyncAfter(deadline: .now() + 30) {
            self.activeProcess?.terminate()
            DispatchQueue.global().asyncAfter(deadline: .now() + 2) {
                if self.activeProcess?.isRunning == true { self.activeProcess?.interrupt() }
                DispatchQueue.global().asyncAfter(deadline: .now() + 2) {
                    if self.activeProcess?.isRunning == true { Darwin.kill(self.activeProcess!.processIdentifier, SIGKILL) }
                }
            }
        }

        let outHandle = outPipe.fileHandleForReading
        let errHandle = errPipe.fileHandleForReading

        outHandle.readabilityHandler = { [weak self] handle in
            let data = handle.availableData
            guard !data.isEmpty, let text = String(data: data, encoding: .utf8) else { return }
            Task { @MainActor [weak self] in
                self?.output.append(text)
            }
        }

        errHandle.readabilityHandler = { [weak self] handle in
            let data = handle.availableData
            guard !data.isEmpty, let text = String(data: data, encoding: .utf8) else { return }
            Task { @MainActor [weak self] in
                self?.output.append(text)
            }
        }

        Task.detached(priority: .userInitiated) {
            do {
                try proc.run()
                proc.waitUntilExit()

                outHandle.readabilityHandler = nil
                errHandle.readabilityHandler = nil

                // Drain remaining data
                let remOut = outHandle.readDataToEndOfFile()
                let remErr = errHandle.readDataToEndOfFile()
                let remOutText = String(data: remOut, encoding: .utf8) ?? ""
                let remErrText = String(data: remErr, encoding: .utf8) ?? ""

                let code = proc.terminationStatus

                await MainActor.run { [weak self] in
                    guard let self = self else { return }
                    if !remOutText.isEmpty { self.output.append(remOutText) }
                    if !remErrText.isEmpty { self.output.append(remErrText) }
                    self.lastExitCode = code
                    self.isRunning = false
                    self.activeProcess = nil
                    self.timer?.invalidate()
                    self.timer = nil

                    if code == 0 {
                        self.output.append("\n✓ Command finished successfully with code 0.\n")
                    } else {
                        self.output.append("\n⚠ Command failed with exit code \(code).\n")
                    }
                    LiquidGlass.haptic(code == 0 ? .alignment : .levelChange)
                }
            } catch {
                outHandle.readabilityHandler = nil
                errHandle.readabilityHandler = nil
                await MainActor.run { [weak self] in
                    guard let self = self else { return }
                    self.output.append("\n✗ Execution Error: \(error.localizedDescription)\n")
                    self.lastExitCode = -1
                    self.isRunning = false
                    self.activeProcess = nil
                    self.timer?.invalidate()
                    self.timer = nil
                    LiquidGlass.haptic(.levelChange)
                }
            }
        }
    }

    public func cancel() {
        guard isRunning, let proc = activeProcess else { return }
        proc.terminate()
        // Do NOT clear isRunning/activeProcess here — the detached
        // completion handler will run shortly (waitUntilExit returns after
        // terminate) and safely nil them out, avoiding a race where the UI
        // reads nil while the task is still dumping output.
        timer?.invalidate()
        timer = nil
        output.append("\n[Process terminated by user]\n")
        LiquidGlass.haptic(.alignment)
    }

    public func clear() {
        output = ""
        lastExitCode = nil
        activeCommand = nil
        executionDuration = 0
        LiquidGlass.haptic(.alignment)
    }
}

// MARK: - Agent Console View

public struct AgentConsoleView: View {
    @ObservedObject var console: AgentConsoleStore
    let workspaceRoot: URL
    let onClose: () -> Void

    public init(
        console: AgentConsoleStore,
        workspaceRoot: URL,
        onClose: @escaping () -> Void
    ) {
        self.console = console
        self.workspaceRoot = workspaceRoot
        self.onClose = onClose
    }

    public var body: some View {
        VStack(spacing: 0) {
            // Console Toolbar
            HStack(spacing: 8) {
                // Command Indicator & Title
                HStack(spacing: 6) {
                    Image(systemName: "terminal.fill")
                        .font(.system(size: 11))
                        .foregroundColor(.accentColor)
                    Text("Agent Console")
                        .font(.system(size: 11.5, weight: .bold))

                    if let cmd = console.activeCommand {
                        Text("•")
                            .foregroundColor(.secondary)
                        Text(cmd)
                            .font(.system(size: 10.5, design: .monospaced))
                            .foregroundColor(.secondary)
                    }
                }

                Spacer()

                // Quick Action Buttons
                Button {
                    console.runCargoTests(repoRoot: workspaceRoot)
                } label: {
                    HStack(spacing: 4) {
                        Image(systemName: "gearshape.2.fill")
                            .font(.system(size: 9))
                            .foregroundColor(.orange)
                        Text("Cargo Tests")
                            .font(.system(size: 10, weight: .medium))
                    }
                    .padding(.horizontal, 6)
                    .padding(.vertical, 3)
                    .background(Capsule().fill(Color.orange.opacity(0.12)))
                }
                .buttonStyle(.plain)
                .disabled(console.isRunning)
                .help("Run cargo test on rust-src backend")
                .accessibilityIdentifier("cargoTestsButton")

                Button {
                    console.runSwiftTests(repoRoot: workspaceRoot)
                } label: {
                    HStack(spacing: 4) {
                        Image(systemName: "swift")
                            .font(.system(size: 9))
                            .foregroundColor(.accentColor)
                        Text("Swift Tests")
                            .font(.system(size: 10, weight: .medium))
                    }
                    .padding(.horizontal, 6)
                    .padding(.vertical, 3)
                    .background(Capsule().fill(Color.accentColor.opacity(0.12)))
                }
                .buttonStyle(.plain)
                .disabled(console.isRunning)
                .help("Run swift test on SwiftUI package")
                .accessibilityIdentifier("swiftTestsButton")

                Button {
                    console.runGitDiff(repoRoot: workspaceRoot)
                } label: {
                    HStack(spacing: 4) {
                        Image(systemName: "arrow.triangle.branch")
                            .font(.system(size: 9))
                            .foregroundColor(.purple)
                        Text("Git Diff")
                            .font(.system(size: 10, weight: .medium))
                    }
                    .padding(.horizontal, 6)
                    .padding(.vertical, 3)
                    .background(Capsule().fill(Color.purple.opacity(0.12)))
                }
                .buttonStyle(.plain)
                .disabled(console.isRunning)
                .help("Inspect live uncommitted git diff")
                .accessibilityIdentifier("gitDiffButton")

                Button {
                    console.runStackHealth(repoRoot: workspaceRoot)
                } label: {
                    HStack(spacing: 4) {
                        Image(systemName: "heart.text.square")
                            .font(.system(size: 9))
                            .foregroundColor(.green)
                        Text("Stack Health")
                            .font(.system(size: 10, weight: .medium))
                    }
                    .padding(.horizontal, 6)
                    .padding(.vertical, 3)
                    .background(Capsule().fill(Color.green.opacity(0.12)))
                }
                .buttonStyle(.plain)
                .disabled(console.isRunning)
                .help("Run lac status check")
                .accessibilityIdentifier("stackHealthButton")

                Divider()
                    .frame(height: 14)
                    .opacity(0.4)

                // Status Badge & Timer
                if console.isRunning {
                    HStack(spacing: 4) {
                        ProgressView()
                            .controlSize(.mini)
                        Text(String(format: "%.1fs", console.executionDuration))
                            .font(.system(size: 10, design: .monospaced))
                            .foregroundColor(.secondary)

                        Button {
                            console.cancel()
                        } label: {
                            Image(systemName: "stop.circle.fill")
                                .font(.system(size: 12))
                                .foregroundColor(.red)
                        }
                        .buttonStyle(.plain)
                        .help("Cancel running command")
                        .accessibilityIdentifier("consoleStopButton")
                    }
                } else if let code = console.lastExitCode {
                    HStack(spacing: 3) {
                        Image(systemName: code == 0 ? "checkmark.circle.fill" : "xmark.circle.fill")
                            .font(.system(size: 10))
                            .foregroundColor(code == 0 ? .green : .red)
                        Text(code == 0 ? "Pass" : "Exit \(code)")
                            .font(.system(size: 9.5, weight: .bold, design: .monospaced))
                            .foregroundColor(code == 0 ? .green : .red)
                    }
                    .padding(.horizontal, 5)
                    .padding(.vertical, 2)
                    .background(Capsule().fill((code == 0 ? Color.green : Color.red).opacity(0.15)))
                }

                // Copy Output
                Button {
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString(console.output, forType: .string)
                    LiquidGlass.haptic(.alignment)
                } label: {
                    Image(systemName: "doc.on.doc")
                        .font(.system(size: 10))
                        .foregroundColor(.secondary)
                }
                .buttonStyle(.plain)
                .help("Copy Console Output")
                .disabled(console.output.isEmpty)

                // Clear Output
                Button {
                    console.clear()
                } label: {
                    Image(systemName: "trash")
                        .font(.system(size: 10))
                        .foregroundColor(.secondary)
                }
                .buttonStyle(.plain)
                .help("Clear Output")
                .disabled(console.output.isEmpty)

                // Close Drawer Button
                Button {
                    onClose()
                } label: {
                    Image(systemName: "chevron.down")
                        .font(.system(size: 10, weight: .semibold))
                        .foregroundColor(.secondary)
                }
                .buttonStyle(.plain)
                .help("Hide Console Drawer (⌥⌘T)")
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 6)
            .background(Color.black.opacity(0.40))
            .overlay(Divider().opacity(0.25), alignment: .bottom)

            // Terminal Output Pane
            ScrollViewReader { proxy in
                ScrollView(.vertical, showsIndicators: true) {
                    VStack(alignment: .leading, spacing: 0) {
                        if console.output.isEmpty {
                            Text("Ready. Click an action button above to run tests, view git diff, or check stack health.")
                                .font(.system(size: 11, design: .monospaced))
                                .foregroundColor(.secondary.opacity(0.6))
                                .padding(10)
                        } else {
                            Text(console.output)
                                .font(.system(size: 11, design: .monospaced))
                                .foregroundColor(.primary.opacity(0.92))
                                .padding(10)
                                .frame(maxWidth: .infinity, alignment: .leading)
                                .textSelection(.enabled)
                        }
                        Color.clear
                            .frame(height: 1)
                            .id("console_bottom")
                    }
                }
                .background(Color(nsColor: .black).opacity(0.70))
                .onChange(of: console.output) { _ in
                    proxy.scrollTo("console_bottom", anchor: .bottom)
                }
            }
        }
        .frame(height: 190)
        .background(Color.black.opacity(0.55))
        .overlay(Divider().opacity(0.3), alignment: .top)
    }
}
