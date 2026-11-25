import SwiftUI
import CowchatKit

struct RootView: View {
    @EnvironmentObject private var store: ConnectionsStore

    var body: some View {
        #if DEBUG
        if ProcessInfo.processInfo.arguments.contains("-accountScreens") {
            AccountScreensHarness()
        } else {
            content
        }
        #else
        content
        #endif
    }

    private var content: some View {
        Group {
            if store.connections.isEmpty {
                SignInView()
            } else {
                MainView()
            }
        }
        .sheet(item: $store.pendingFriendLink) { FriendLinkSheet(route: $0) }
    }
}
