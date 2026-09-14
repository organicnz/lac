import AppKit
import SwiftUI

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    static weak var mainWindow: NSWindow?

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.regular)
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) {
            Self.showMainWindow()
        }
    }

    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        Self.showMainWindow()
        return true
    }

    static func showMainWindow() {
        NSApp.activate(ignoringOtherApps: true)
        if let win = mainWindow {
            win.makeKeyAndOrderFront(nil)
            win.orderFrontRegardless()
            return
        }
        for window in NSApp.windows {
            if window.canBecomeMain {
                mainWindow = window
                window.makeKeyAndOrderFront(nil)
                window.orderFrontRegardless()
                return
            }
        }
    }
}

@main
struct LACStudioApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) var appDelegate
    // Single shared manager: the window and the menu bar extra observe
    // the same fetch state instead of polling the router twice.
    @StateObject private var network = NetworkManager()
    @StateObject private var chatStore = ChatStore()

    var body: some Scene {
        WindowGroup("Loop LAC Studio") {
            MainAppView()
                .frame(minWidth: 960, minHeight: 640)
                .background(VisualEffectView().ignoresSafeArea())
                .background(WindowGlassAccessor())
                .environmentObject(network)
                .environmentObject(chatStore)
        }
        .windowResizability(.contentSize)

        MenuBarExtra("Loop LAC Studio", systemImage: "infinity") {
            MenuBarView()
                .environmentObject(network)
        }
    }
}
