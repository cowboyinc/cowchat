import BackgroundTasks
import SwiftUI
import CowchatKit

@main
struct CowchatApp: App {
    @StateObject private var store = CowchatApp.makeStore()

    private static func makeStore() -> ConnectionsStore {
        #if DEBUG
        if DemoMode.isOn { return DemoMode.makeStore() }
        #endif
        return ConnectionsStore()
    }
    @Environment(\.scenePhase) private var scenePhase

    init() { GallopChrome.apply() }

    var body: some Scene {
        WindowGroup {
            RootView()
                .environmentObject(store)
                .tint(Palette.nugget500)
                .task {
                    #if DEBUG
                    if DemoMode.isOn { await DemoMode.signIn(store) }
                    #endif
                    await store.restore()
                }
                .onOpenURL { store.openFriendLink($0) }
                .onChange(of: scenePhase) { _, phase in
                    switch phase {
                    case .active: store.sceneBecameActive()
                    case .background:
                        store.sceneEnteredBackground()
                        Self.scheduleRefresh()
                    default: break
                    }
                }
        }
        // iOS runs this now and then while the app is closed. It reads the Inbox over HTTPS and notifies about
        // what is new. It is the nearest thing to push the phone can do without the server sending pushes.
        .backgroundTask(.appRefresh(Self.refreshID)) {
            await Self.scheduleRefresh()
            await store.refreshAccounts()
            // When the refresh last ran, for checking on a device that iOS does run it.
            UserDefaults.standard.set(Date(), forKey: "CowchatiOS.lastBackgroundRefresh")
        }
    }

    static let refreshID = "inc.cowboy.cowchat.ios.refresh"

    static func scheduleRefresh() {
        let request = BGAppRefreshTaskRequest(identifier: refreshID)
        request.earliestBeginDate = Date(timeIntervalSinceNow: 15 * 60)
        try? BGTaskScheduler.shared.submit(request)
    }
}
