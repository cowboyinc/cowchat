import AppKit
import Combine

/// The Inbox's unread count on the Dock's icon, with nothing at zero or while signed out. Cowchat puts nothing in the
/// menu bar (Patrick, 2 November): this badge and a Mac notification for each new item cover what its panel showed.
@MainActor
final class InboxDockBadge {
    private var observer: AnyCancellable?

    func start(inbox: InboxStore) {
        guard observer == nil else { return }
        observer = inbox.objectWillChange
            .receive(on: RunLoop.main)
            .sink { [weak inbox] _ in
                guard let inbox else { return }
                NSApp.dockTile.badgeLabel = Self.label(for: inbox)
            }
        NSApp.dockTile.badgeLabel = Self.label(for: inbox)
    }

    /// The badge's label: the unread count, or nil at zero or while signed out.
    static func label(for inbox: InboxStore) -> String? {
        let count = inbox.account == nil ? 0 : inbox.unreadCount
        return count > 0 ? "\(count)" : nil
    }
}
