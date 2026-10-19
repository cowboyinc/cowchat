import Combine
import Foundation
import Sparkle

@MainActor
final class SoftwareUpdater: ObservableObject {
    @Published private(set) var canCheckForUpdates = false
    @Published private(set) var automaticallyChecksForUpdates = false
    @Published private(set) var automaticallyDownloadsUpdates = false

    private var controller: SPUStandardUpdaterController?

    var isAvailable: Bool { controller != nil }

    init(bundle: Bundle = .main) {
        #if DEBUG
        return
        #else
        guard bundle.bundleURL.pathExtension == "app",
              bundle.bundleIdentifier == "inc.cowboy.cowchat"
        else { return }

        let controller = SPUStandardUpdaterController(
            startingUpdater: false,
            updaterDelegate: nil,
            userDriverDelegate: nil
        )
        self.controller = controller
        controller.updater.publisher(for: \.canCheckForUpdates)
            .receive(on: RunLoop.main)
            .assign(to: &$canCheckForUpdates)
        controller.updater.publisher(for: \.automaticallyChecksForUpdates)
            .receive(on: RunLoop.main)
            .assign(to: &$automaticallyChecksForUpdates)
        controller.updater.publisher(for: \.automaticallyDownloadsUpdates)
            .receive(on: RunLoop.main)
            .assign(to: &$automaticallyDownloadsUpdates)
        #endif
    }

    func start() {
        controller?.startUpdater()
    }

    func checkForUpdates() {
        guard canCheckForUpdates else { return }
        controller?.checkForUpdates(nil)
    }

    func setAutomaticallyChecksForUpdates(_ enabled: Bool) {
        controller?.updater.automaticallyChecksForUpdates = enabled
    }

    func setAutomaticallyDownloadsUpdates(_ enabled: Bool) {
        controller?.updater.automaticallyDownloadsUpdates = enabled
    }
}
