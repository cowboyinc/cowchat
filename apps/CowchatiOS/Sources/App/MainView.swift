import SwiftUI
import CowchatKit

enum AppTab: Hashable {
    case rooms, inbox, people, you
}

/// Where the app is: which tab, and which room is open. Any screen can open a room through it.
@MainActor
final class Navigator: ObservableObject {
    @Published var tab = AppTab.rooms
    @Published var roomPath: [RoomRef] = []

    func open(_ room: RoomRef) {
        tab = .rooms
        roomPath = [room]
    }
}

/// The signed-in app: Rooms, then Inbox and People for a connection signed in to an account, then You.
struct MainView: View {
    @EnvironmentObject private var store: ConnectionsStore
    @StateObject private var navigator = Navigator()

    private var showsStorageError: Binding<Bool> {
        Binding(get: { store.storageError != nil }, set: { if !$0 { store.storageError = nil } })
    }

    var body: some View {
        TabView(selection: $navigator.tab) {
            RoomListView()
                .tabItem { Label("Rooms", systemImage: "bubble.left.and.bubble.right") }
                .tag(AppTab.rooms)
            if !store.accounts.isEmpty {
                InboxView(showsDone: false) { navigator.open($0) }
                    .tabItem { Label("Inbox", systemImage: "tray") }
                    .tag(AppTab.inbox)
                    .badge(store.inboxUnread)
                PeopleView { navigator.open($0) }
                    .tabItem { Label("People", systemImage: "person.2") }
                    .tag(AppTab.people)
            }
            YouView()
                .tabItem { Label("You", systemImage: "person.crop.circle") }
                .tag(AppTab.you)
        }
        .environmentObject(navigator)
        .alert(store.storageError ?? "", isPresented: showsStorageError) {
            Button("OK", role: .cancel) {}
        }
        .background { ForEach(store.accounts, id: \.connectionID) { SetupGate(account: $0) } }
        .onChange(of: store.accounts.isEmpty) { _, hasNone in
            // Inbox and People go when the last account does.
            if hasNone, navigator.tab == .inbox || navigator.tab == .people { navigator.tab = .rooms }
        }
        .task {
            // The account side has no live events, so it is re-read while the app is showing.
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(30))
                await store.refreshAccounts()
            }
        }
    }
}
