import AppKit
import SwiftUI
import XCTest
@testable import CowchatMac

final class RoomLeaseTests: XCTestCase {
    @MainActor
    func testLeaseRowRenders() async throws {
        let lease = try JSONDecoder().decode(RoomLease.self, from: Data(#"{"name":"deploy:canyon","generation":3,"holder_name":"Coordinator","task_state":"waiting_for_human","detail":"Waiting for deployment approval","progress":"Candidate built and tests passed","last_heartbeat_at":1000,"last_progress_at":500,"overdue_at":1300,"expires_at":1420}"#.utf8))
        let host = NSHostingView(rootView: RoomLeaseRow(lease: lease, date: Date(timeIntervalSince1970: 1100)).padding(24).frame(width: 640, height: 200).background(Color(nsColor: .windowBackgroundColor)))
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 640, height: 200), styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        defer { window.close() }
        window.contentView = host; window.orderFront(nil)
        try await Task.sleep(nanoseconds: 200_000_000)
        window.display(); host.layoutSubtreeIfNeeded()
        let bitmap = try XCTUnwrap(host.bitmapImageRepForCachingDisplay(in: host.bounds))
        host.cacheDisplay(in: host.bounds, to: bitmap)
        let png = try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
        XCTAssertGreaterThan(png.count, 1000)
        try png.write(to: URL(fileURLWithPath: "/tmp/cowchat-lease-preview.png"))
    }
    func testHeartbeatAndProgressRemainIndependentAndStatusAges() throws {
        let data = Data(#"{"name":"deploy:canyon","generation":3,"holder_name":"Coordinator","task_state":"waiting_for_human","detail":"Approve deploy","progress":"Build passed","last_heartbeat_at":1000,"last_progress_at":500,"overdue_at":1300,"expires_at":1420,"released_at":null}"#.utf8)
        let lease = try JSONDecoder().decode(RoomLease.self, from: data)
        XCTAssertEqual(lease.status(at: Date(timeIntervalSince1970: 1299)), "Waiting for human")
        XCTAssertEqual(lease.status(at: Date(timeIntervalSince1970: 1300)), "Overdue")
        XCTAssertEqual(lease.status(at: Date(timeIntervalSince1970: 1420)), "Expired")
        XCTAssertEqual(lease.lastProgressAt, 500)
        XCTAssertEqual(lease.generation, 3)
    }
}
