import AppKit
import SwiftUI

@MainActor
final class CowchatAppDelegate: NSObject, NSApplicationDelegate {
    private var screenObserver: NSObjectProtocol?
    private var terminationTask: Task<Void, Never>?
    private var terminationShutdownCompleted = false
    var onTerminationRequested: (@MainActor () async -> Void)?
    var replyToApplicationShouldTerminate: @MainActor (NSApplication, Bool) -> Void = {
        application, shouldTerminate in
        application.reply(toApplicationShouldTerminate: shouldTerminate)
    }

    func applicationWillFinishLaunching(_ notification: Notification) {
        NSApplication.shared.setActivationPolicy(.regular)
        if let icon = Self.applicationIcon() {
            NSApplication.shared.applicationIconImage = icon
        }
    }

    nonisolated static func applicationIcon() -> NSImage? {
        if let iconURL = Bundle.main.url(forResource: "Cowchat", withExtension: "icns") {
            return NSImage(contentsOf: iconURL)
        }

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
            Task { @MainActor [weak self] in
                self?.restoreWindowsToVisibleScreens()
            }
        }
        DispatchQueue.main.async {
            NSApplication.shared.activate(ignoringOtherApps: true)
            NSApplication.shared.windows.first?.makeKeyAndOrderFront(nil)
        }
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        true
    }

    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        if terminationShutdownCompleted { return .terminateNow }
        guard let onTerminationRequested else { return .terminateNow }

        if terminationTask == nil {
            let reply = replyToApplicationShouldTerminate
            terminationTask = Task { @MainActor [weak self] in
                await onTerminationRequested()
                guard let self else {
                    reply(sender, true)
                    return
                }
                terminationShutdownCompleted = true
                terminationTask = nil
                reply(sender, true)
            }
        }
        return .terminateLater
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

    nonisolated static func recoveredFrame(_ frame: CGRect, visibleFrames: [CGRect]) -> CGRect {
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
    @AppStorage(CowchatOnboarding.completedVersionKey) private var completedOnboardingVersion = 0

    var body: some Scene {
        WindowGroup {
            Group {
                if completedOnboardingVersion < CowchatOnboarding.currentVersion {
                    CowchatOnboardingView {
                        completedOnboardingVersion = CowchatOnboarding.currentVersion
                    }
                } else {
                    ContentView {
                        completedOnboardingVersion = 0
                    }
                }
            }
            .environmentObject(store)
            .tint(SemanticColor.buttonPrimaryDefault)
            .task {
                appDelegate.onTerminationRequested = {
                    await store.shutdownOwnedLocalServerForAppTermination()
                }
                store.start()
            }
        }
        .defaultSize(width: 1080, height: 740)
        .windowToolbarStyle(.unified(showsTitle: false))
        .commands {
            CommandGroup(after: .newItem) {
                Button("New Room") { store.presentCreateRoom() }
                    .keyboardShortcut("n", modifiers: .command)
                    .disabled(completedOnboardingVersion < CowchatOnboarding.currentVersion)
            }
            CommandGroup(after: .sidebar) {
                Button("Toggle Sidebar") {
                    NotificationCenter.default.post(name: .cowchatToggleSidebar, object: nil)
                }
                .keyboardShortcut("s", modifiers: [.control, .command])
            }
        }
    }
}

extension Notification.Name {
    /// ⌃⌘S — macOS reserves this for sidebar toggling; nothing binds it once
    /// the shell owns its own fixed column (cowboy-app pattern).
    static let cowchatToggleSidebar = Notification.Name("CowchatMac.toggleSidebar")
}
