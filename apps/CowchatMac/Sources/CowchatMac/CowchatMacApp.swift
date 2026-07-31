import AppKit
import SwiftUI

final class CowchatAppDelegate: NSObject, NSApplicationDelegate {
    private var screenObserver: NSObjectProtocol?

    func applicationWillFinishLaunching(_ notification: Notification) {
        NSApplication.shared.setActivationPolicy(.regular)
        if let icon = Self.applicationIcon() {
            NSApplication.shared.applicationIconImage = icon
        }
    }

    static func applicationIcon() -> NSImage? {
        if let iconURL = Bundle.main.url(forResource: "CowchatIcon", withExtension: "png") {
            return NSImage(contentsOf: iconURL)
        }

        #if DEBUG
        if let iconURL = Bundle.module.url(forResource: "CowchatIcon", withExtension: "png") {
            return NSImage(contentsOf: iconURL)
        }
        #endif

        return nil
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        screenObserver = NotificationCenter.default.addObserver(
            forName: NSApplication.didChangeScreenParametersNotification,
            object: nil,
            queue: .main
        ) { [weak self] _ in
            self?.restoreWindowsToVisibleScreens()
        }
        DispatchQueue.main.async {
            NSApplication.shared.activate(ignoringOtherApps: true)
            NSApplication.shared.windows.first?.makeKeyAndOrderFront(nil)
        }
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        true
    }

    deinit {
        if let screenObserver { NotificationCenter.default.removeObserver(screenObserver) }
    }

    func restoreWindowsToVisibleScreens() {
        let visibleFrames = NSScreen.screens.map(\.visibleFrame)
        guard !visibleFrames.isEmpty else { return }
        for window in NSApplication.shared.windows {
            let recovered = Self.recoveredFrame(window.frame, visibleFrames: visibleFrames)
            if recovered != window.frame { window.setFrame(recovered, display: true) }
        }
    }

    static func recoveredFrame(_ frame: CGRect, visibleFrames: [CGRect]) -> CGRect {
        guard let target = visibleFrames.first(where: { $0.intersects(frame) }) ?? visibleFrames.first else {
            return frame
        }
        let size = CGSize(
            width: min(frame.width, target.width),
            height: min(frame.height, target.height)
        )
        let origin: CGPoint
        if target.intersects(frame) {
            origin = CGPoint(
                x: min(max(frame.minX, target.minX), target.maxX - size.width),
                y: min(max(frame.minY, target.minY), target.maxY - size.height)
            )
        } else {
            origin = CGPoint(x: target.midX - size.width / 2, y: target.midY - size.height / 2)
        }
        return CGRect(origin: origin, size: size)
    }
}

@main
struct CowchatMacApp: App {
    @NSApplicationDelegateAdaptor(CowchatAppDelegate.self) private var appDelegate
    @StateObject private var store = ChatStore()

    var body: some Scene {
        WindowGroup {
            ContentView()
                .environmentObject(store)
                .tint(Color(red: 0.16, green: 0.61, blue: 0.88))
        }
        .windowStyle(.titleBar)
        .defaultSize(width: 1120, height: 720)
        .commands {
            CommandGroup(after: .newItem) {
                Button("New Room") { store.isCreateRoomPresented = true }
                    .keyboardShortcut("n", modifiers: .command)
            }
        }
    }
}
