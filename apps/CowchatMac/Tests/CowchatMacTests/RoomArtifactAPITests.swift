import Foundation
import XCTest
@testable import CowchatMac

final class RoomArtifactAPITests: XCTestCase {
    func testShareLinkUsesOnlyReadCapability() throws {
        let profile = try ConnectionProfile.cowchatCloud(urlString: "wss://chat.example/prefix/ws", apiKey: "writer-secret")
        let id = "91378ee3-c910-4330-afef-c017381101b2"
        let url = try RoomArtifactAPI.shareURL(profile: profile, publicID: id)
        XCTAssertEqual(url.absoluteString, "https://chat.example/prefix/a/\(id)")
        XCTAssertFalse(url.absoluteString.contains(profile.apiKey))
        XCTAssertThrowsError(try RoomArtifactAPI.shareURL(profile: .local, publicID: id))
        XCTAssertThrowsError(try RoomArtifactAPI.shareURL(profile: profile, publicID: "../api/keys"))
    }

    func testMetadataAcceptsServerFractionalTimestamp() throws {
        let data = Data(#"{"title":"Engineering","revision":3,"public_id":"91378ee3-c910-4330-afef-c017381101b2","updated_at":"2026-10-17T03:00:01.125+00:00","data":{"ignored":true},"view":"root = Text(\"Hi\")"}"#.utf8)
        let artifact = try JSONDecoder().decode(RoomArtifactMetadata.self, from: data)
        XCTAssertEqual(artifact.title, "Engineering")
        XCTAssertEqual(artifact.revision, 3)
        XCTAssertNotNil(artifact.updatedDate)
    }

    func testAuthenticatedRequestDoesNotPutWriterKeyInURL() async throws {
        let profile = try ConnectionProfile.cowchatCloud(urlString: "wss://chat.example/ws", apiKey: "writer-secret")
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [ArtifactFixtureURLProtocol.self]
        let session = URLSession(configuration: configuration)
        defer { session.invalidateAndCancel(); ArtifactFixtureURLProtocol.handler = nil }
        ArtifactFixtureURLProtocol.handler = { request in
            XCTAssertEqual(request.url?.absoluteString, "https://chat.example/api/rooms/room-1/artifact")
            XCTAssertEqual(request.value(forHTTPHeaderField: "x-cowchat-key"), profile.apiKey)
            XCTAssertEqual(request.httpMethod, "GET")
            return (404, Data())
        }
        let artifact = try await RoomArtifactAPI(profile: profile, session: session).get(roomID: "room-1")
        XCTAssertNil(artifact)
    }
}

private final class ArtifactFixtureURLProtocol: URLProtocol {
    static var handler: ((URLRequest) throws -> (Int, Data))?
    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        do {
            let (status, data) = try Self.handler!(request)
            let response = HTTPURLResponse(url: request.url!, statusCode: status,
                                           httpVersion: nil, headerFields: ["Content-Type": "application/json"])!
            client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
            client?.urlProtocol(self, didLoad: data)
            client?.urlProtocolDidFinishLoading(self)
        } catch {
            client?.urlProtocol(self, didFailWithError: error)
        }
    }
    override func stopLoading() {}
}
