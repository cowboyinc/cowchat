import AppKit
import SwiftUI
import Foundation
import XCTest
@testable import CowchatMac

final class SlackAPITests: XCTestCase {
    func testSlackIsUnavailableForLocalRooms() {
        XCTAssertThrowsError(try SlackAPI.endpoint(profile: .local, path: ["connect"]))
    }
    func testAuthenticatedGlobalEndpointUsesHTTPSAndEncodesRoomIDs() throws {
        let profile = try ConnectionProfile.cowchatCloud(urlString: "wss://chat.example/ws", apiKey: "secret")
        let url = try SlackAPI.endpoint(profile: profile, path: ["rooms", "room name"])
        XCTAssertEqual(url.scheme, "https")
        XCTAssertEqual(url.host, "chat.example")
        XCTAssertEqual(url.path, "/integrations/slack/rooms/room name")
        XCTAssertNil(url.query)
        XCTAssertFalse(url.absoluteString.contains("secret"))
    }
    func testSlackStatusDecodesPauseHealthAndQueueWithoutSecrets() throws {
        let data = Data(#"{"connected":true,"channel_name":"project","paused":true,"error":"Waiting for Slack","queued":3}"#.utf8)
        let status = try JSONDecoder().decode(SlackRoomStatus.self, from: data)
        XCTAssertEqual(status.channelName, "project")
        XCTAssertEqual(status.paused, true)
        XCTAssertEqual(status.queued, 3)
        XCTAssertEqual(status.error, "Waiting for Slack")
    }
}

private final class SlackFixtureURLProtocol: URLProtocol {
    static var handler: ((URLRequest) throws -> (Int, Data))?
    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        do {
            let (status, data) = try Self.handler!(request)
            let response = HTTPURLResponse(url: request.url!, statusCode: status, httpVersion: nil, headerFields: ["Content-Type": "application/json"])!
            client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
            client?.urlProtocol(self, didLoad: data)
            client?.urlProtocolDidFinishLoading(self)
        } catch { client?.urlProtocol(self, didFailWithError: error) }
    }
    override func stopLoading() {}
}

extension SlackAPITests {
    func testRoomConnectionSendsSecretOnlyToAuthenticatedGlobalEndpoint() async throws {
        let profile = try ConnectionProfile.cowchatCloud(urlString: "wss://chat.example/ws", apiKey: "test-key")
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [SlackFixtureURLProtocol.self]
        let session = URLSession(configuration: configuration)
        defer { session.invalidateAndCancel(); SlackFixtureURLProtocol.handler = nil }
        SlackFixtureURLProtocol.handler = { request in
            XCTAssertEqual(request.url?.absoluteString, "https://chat.example/integrations/slack/rooms/one")
            XCTAssertEqual(request.httpMethod, "POST")
            XCTAssertEqual(request.value(forHTTPHeaderField: "x-cowchat-key"), "test-key")
            // URLSession may materialize the JSON as a body stream.
            if let data = request.httpBody {
                let body = try JSONSerialization.jsonObject(with: data) as! [String: String]
                XCTAssertEqual(body["room_secret"], "room-secret")
                XCTAssertEqual(body["channel_id"], "C1")
            }
            return (200, Data(#"{"connected":true,"channel_name":"project"}"#.utf8))
        }
        let status: SlackRoomStatus = try await SlackAPI(profile: profile, session: session).request(["rooms", "one"], method: "POST", body: ["channel_id": "C1", "room_secret": "room-secret"])
        XCTAssertTrue(status.connected)
    }

    func testServerSetupErrorsReachTheRoomSettings() async throws {
        let profile = try ConnectionProfile.cowchatCloud(urlString: "wss://chat.example/ws", apiKey: "test-key")
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [SlackFixtureURLProtocol.self]
        let session = URLSession(configuration: configuration)
        defer { session.invalidateAndCancel(); SlackFixtureURLProtocol.handler = nil }
        SlackFixtureURLProtocol.handler = { _ in (503, Data(#"{"error":"Slack is not configured on this server."}"#.utf8)) }
        do {
            let _: SlackRoomStatus = try await SlackAPI(profile: profile, session: session).request(["rooms", "one"])
            XCTFail("Expected setup error")
        } catch { XCTAssertEqual(error.localizedDescription, "Slack is not configured on this server.") }
    }
}

extension SlackAPITests {
    @MainActor
    func testConnectedRoomSettingsRenderWithServerHealth() async throws {
        let profile = try ConnectionProfile.cowchatCloud(urlString: "wss://chat.example/ws", apiKey: "test-key")
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [SlackFixtureURLProtocol.self]
        let session = URLSession(configuration: configuration)
        let received = expectation(description: "Room settings fetched server status")
        SlackFixtureURLProtocol.handler = { request in
            XCTAssertEqual(request.url?.path, "/integrations/slack/rooms/one")
            received.fulfill()
            return (200, Data(#"{"connected":true,"channel_name":"project","paused":false,"queued":2,"error":"Waiting for Slack to reconnect"}"#.utf8))
        }
        defer { session.invalidateAndCancel(); SlackFixtureURLProtocol.handler = nil }
        let defaults = UserDefaults(suiteName: "SlackRoomUITest-\(UUID().uuidString)")!
        let store = ChatStore(connection: CowchatConnection(profile: profile), defaults: defaults, connectionProfile: profile)
        let room = Room(roomID: "one", name: "Planning", description: nil, parentID: nil, createdAt: "", createdBy: nil, visibility: "private", lastActivity: nil, memberCount: 2, encrypted: false)
        let host = NSHostingView(rootView: SlackRoomView(room: room, session: session).environmentObject(store))
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 500, height: 380), styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        defer { window.close() }
        window.contentView = host
        window.display()
        await fulfillment(of: [received], timeout: 5)
        // Let the decoded response update SwiftUI before capturing the fixture.
        try await Task.sleep(nanoseconds: 100_000_000)
        host.layoutSubtreeIfNeeded()
        let bitmap = try XCTUnwrap(host.bitmapImageRepForCachingDisplay(in: host.bounds))
        host.cacheDisplay(in: host.bounds, to: bitmap)
        let png = try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
        XCTAssertGreaterThan(png.count, 1000)
        try png.write(to: URL(fileURLWithPath: "/tmp/cowchat-slack-room-settings.png"))
    }
}

extension SlackAPITests {
    func testExternalDisplayMetadataDefaultsToTrueAndPreservesFalse() throws {
        let decoder = JSONDecoder()
        XCTAssertTrue(try decoder.decode(MessageMetadata.self, from: Data("{}".utf8)).shouldDisplayExternal)
        let hidden = try decoder.decode(MessageMetadata.self, from: Data(#"{"display_external":false,"type":"chart"}"#.utf8))
        XCTAssertFalse(hidden.shouldDisplayExternal)
        let encoded = try JSONEncoder().encode(hidden)
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: encoded) as? [String: Any])
        XCTAssertEqual(object["display_external"] as? Bool, false)
        XCTAssertEqual(object["type"] as? String, "chart")
    }
}
