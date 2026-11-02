import AppKit
import Combine
import SwiftUI

/// The menu bar item and the Dock badge for the Inbox: the bell with the unread count beside it, and the count on the
/// Dock's icon. Pressing the bell opens the panel at the bell's measure (`InboxPanel`). Both show nothing until the
/// person is signed in, and the count shows nothing at zero.
@MainActor
final class InboxMenuBar: NSObject, NSPopoverDelegate {
    private var statusItem: NSStatusItem?
    private let popover = NSPopover()
    private var observers: Set<AnyCancellable> = []
    private weak var workspace: WorkspaceStore?

    func start(workspace: WorkspaceStore) {
        guard self.workspace == nil else { return }
        self.workspace = workspace
        let inbox = workspace.inbox
        popover.behavior = .transient
        popover.delegate = self
        popover.contentViewController = NSHostingController(rootView: InboxPanelRoot(
            inbox: inbox, home: workspace.homeSessions,
            open: { [weak self] item in self?.open(item) },
            openInbox: { [weak self] in self?.openInbox() }
        ).environmentObject(workspace).defaultAppStorage(inbox.defaults))

        inbox.objectWillChange
            .receive(on: RunLoop.main)
            .sink { [weak self] _ in self?.update() }
            .store(in: &observers)
        inbox.$isPanelShown
            .removeDuplicates()
            .receive(on: RunLoop.main)
            .sink { [weak self] shown in self?.setPanel(shown: shown) }
            .store(in: &observers)
        update()
    }

    /// Whether the panel is on screen, for the tour.
    var isPanelShown: Bool { popover.isShown }

    private func update() {
        guard let inbox = workspace?.inbox else { return }
        let count = inbox.account == nil ? 0 : inbox.unreadCount
        NSApp.dockTile.badgeLabel = count > 0 ? "\(count)" : nil
        guard inbox.account != nil else {
            if let statusItem { NSStatusBar.system.removeStatusItem(statusItem) }
            statusItem = nil
            popover.performClose(nil)
            return
        }
        let item = statusItem ?? makeStatusItem()
        item.button?.title = count > 0 ? "\(count)" : ""
        // copy:
        item.button?.setAccessibilityLabel(count > 0 ? "Inbox, \(count) unread" : "Inbox")
    }

    private func makeStatusItem() -> NSStatusItem {
        let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
        if let button = item.button {
            let image = GallopIcon.bell.templateImage
            image?.size = NSSize(width: 16, height: 16)
            button.image = image ?? NSImage(systemSymbolName: "bell", accessibilityDescription: nil)
            button.imagePosition = .imageLeading
            // copy:
            button.toolTip = "Inbox"
            button.target = self
            button.action = #selector(toggle)
        }
        statusItem = item
        return item
    }

    @objc private func toggle() {
        guard let inbox = workspace?.inbox else { return }
        inbox.isPanelShown = !popover.isShown
    }

    private func setPanel(shown: Bool) {
        if shown {
            guard !popover.isShown, let button = statusItem?.button else { return }
            popover.show(relativeTo: button.bounds, of: button, preferredEdge: .minY)
            popover.contentViewController?.view.window?.makeKey()
        } else if popover.isShown {
            popover.performClose(nil)
        }
    }

    func popoverDidClose(_ notification: Notification) {
        workspace?.inbox.isPanelShown = false
    }

    /// An item in the panel does what its Inbox row does, in the app's window.
    private func open(_ item: InboxItem) {
        guard let workspace else { return }
        workspace.inbox.isPanelShown = false
        bringWindowForward()
        if item.isOpenInvitation {
            workspace.show(.inbox)
            workspace.inbox.joining = item
        } else {
            Task { await workspace.open(item) }
        }
    }

    private func openInbox() {
        guard let workspace else { return }
        workspace.inbox.isPanelShown = false
        bringWindowForward()
        workspace.show(.inbox)
    }

    private func bringWindowForward() {
        NSApp.activate(ignoringOtherApps: true)
        NSApp.windows.first { $0.canBecomeMain && !($0 is NSPanel) }?.makeKeyAndOrderFront(nil)
    }
}

/// The panel with the app's own appearance setting, as the window takes it.
private struct InboxPanelRoot: View {
    @ObservedObject var inbox: InboxStore
    @ObservedObject var home: HomeSessions
    let open: (InboxItem) -> Void
    let openInbox: () -> Void
    @AppStorage("CowchatMac.appearance") private var appearance = "system"

    var body: some View {
        InboxPanel(inbox: inbox, home: home, open: open, openInbox: openInbox)
            .preferredColorScheme(appearance == "light" ? .light : appearance == "dark" ? .dark : nil)
    }
}
