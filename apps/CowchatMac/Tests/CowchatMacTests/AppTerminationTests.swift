import AppKit
import XCTest
@testable import CowchatMac

@MainActor
private final class AppTerminationProbe {
    var shutdownCallCount = 0
    var replies: [Bool] = []
    private var shutdownContinuation: CheckedContinuation<Void, Never>?

    func shutdown() async {
        shutdownCallCount += 1
        await withCheckedContinuation { continuation in
            shutdownContinuation = continuation
        }
    }

    func releaseShutdown() {
        shutdownContinuation?.resume()
        shutdownContinuation = nil
    }
}

final class AppTerminationTests: XCTestCase {
    @MainActor
    func testTerminationWaitsCoalescesAndRepliesExactlyOnce() async {
        let delegate = CowchatAppDelegate()
        let probe = AppTerminationProbe()
        delegate.onTerminationRequested = { await probe.shutdown() }
        delegate.replyToApplicationShouldTerminate = { _, shouldTerminate in
            probe.replies.append(shouldTerminate)
        }

        let application = NSApplication.shared
        XCTAssertEqual(delegate.applicationShouldTerminate(application), .terminateLater)
        XCTAssertEqual(delegate.applicationShouldTerminate(application), .terminateLater)

        for _ in 0..<100 where probe.shutdownCallCount == 0 {
            await Task.yield()
        }
        XCTAssertEqual(probe.shutdownCallCount, 1)
        XCTAssertTrue(probe.replies.isEmpty)

        probe.releaseShutdown()
        for _ in 0..<100 where probe.replies.isEmpty {
            await Task.yield()
        }

        XCTAssertEqual(probe.replies, [true])
        XCTAssertEqual(delegate.applicationShouldTerminate(application), .terminateNow)
        XCTAssertEqual(probe.shutdownCallCount, 1)
        XCTAssertEqual(probe.replies, [true])
    }
}
