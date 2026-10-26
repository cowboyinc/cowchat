import Foundation
import XCTest
@testable import CowchatMac

@MainActor
private final class ScriptedWebSocketTask: CowchatWebSocketTaskProtocol {
    private var inbound: [URLSessionWebSocketTask.Message] = []
    private var receiver: CheckedContinuation<URLSessionWebSocketTask.Message, Error>?
    private(set) var sentFrames: [[String: Any]] = []
    private(set) var sentTexts: [String] = []
    private(set) var wasResumed = false
    private(set) var wasCancelled = false
    var onSend: (([String: Any]) -> Void)?

    func resume() {
        wasResumed = true
    }

    func cancel(with _: URLSessionWebSocketTask.CloseCode, reason _: Data?) {
        wasCancelled = true
        receiver?.resume(throwing: URLError(.cancelled))
        receiver = nil
    }

    func send(_ message: URLSessionWebSocketTask.Message) async throws {
        guard case .string(let text) = message,
              let frame = try JSONSerialization.jsonObject(with: Data(text.utf8)) as? [String: Any],
              let type = frame["type"] as? String else {
            throw CowchatConnectionError.invalidResponse
        }
        sentTexts.append(text)
        sentFrames.append(frame)
        onSend?(frame)

        guard let id = frame["id"] as? String else { return }
        switch type {
        case "ping":
            enqueue(frame: [
                "id": UUID().uuidString,
                "reply_to": id,
                "type": "pong",
                "payload": [:],
            ])
        case "register":
            enqueue(frame: [
                "id": UUID().uuidString,
                "reply_to": id,
                "type": "ok",
                "payload": [
                    "agent_id": "cloud-agent",
                    "rooms": ["restored-room"],
                ],
            ])
        default:
            break
        }
    }

    func receive() async throws -> URLSessionWebSocketTask.Message {
        if !inbound.isEmpty { return inbound.removeFirst() }
        return try await withCheckedThrowingContinuation { continuation in
            receiver = continuation
        }
    }

    func enqueue(frame: [String: Any]) {
        let data = try! JSONSerialization.data(withJSONObject: frame)
        enqueue(.string(String(decoding: data, as: UTF8.self)))
    }

    func enqueue(text: String) {
        enqueue(.string(text))
    }

    private func enqueue(_ message: URLSessionWebSocketTask.Message) {
        if let receiver {
            self.receiver = nil
            receiver.resume(returning: message)
        } else {
            inbound.append(message)
        }
    }
}

final class CowchatWebSocketTransportTests: XCTestCase {
    @MainActor
    func testCloudRegisterIsFirstFrameAndCarriesKey() async throws {
        let task = ScriptedWebSocketTask()
        let profile = try ConnectionProfile.cowchatCloud(
            urlString: "wss://cloud.example/ws",
            apiKey: "cloud-registration-key"
        )
        var requestedURL: URL?
        let connection = CowchatConnection(profile: profile) { url in
            requestedURL = url
            return task
        }

        try await connection.connect()
        XCTAssertTrue(task.sentFrames.isEmpty)
        let registration = try await connection.register(name: "Cowchat Mac", agentID: "mac-1")

        XCTAssertEqual(requestedURL?.absoluteString, "wss://cloud.example/ws")
        XCTAssertTrue(task.wasResumed)
        XCTAssertEqual(task.sentFrames.map { $0["type"] as? String }, ["register"])
        XCTAssertFalse(task.sentTexts[0].hasSuffix("\n"))
        let registerPayload = task.sentFrames[0]["payload"] as? [String: Any]
        XCTAssertEqual(registerPayload?["key"] as? String, "cloud-registration-key")
        XCTAssertEqual(registration.agentID, "cloud-agent")
        XCTAssertEqual(registration.restoredRoomIDs, ["restored-room"])

        connection.disconnect()
        XCTAssertTrue(task.wasCancelled)
    }

    @MainActor
    func testCloudAnswersApplicationPingWithPong() async throws {
        let task = ScriptedWebSocketTask()
        let profile = try ConnectionProfile.cowchatCloud(
            urlString: "wss://cloud.example/ws",
            apiKey: "cloud-key"
        )
        let connection = CowchatConnection(profile: profile) { _ in task }
        try await connection.connect()
        _ = try await connection.register(name: "Cowchat Mac", agentID: "mac-ping")

        let pongSent = expectation(description: "application pong sent")
        task.onSend = { frame in
            if frame["type"] as? String == "pong",
               frame["reply_to"] as? String == "server-ping" {
                pongSent.fulfill()
            }
        }
        task.enqueue(frame: [
            "id": "server-ping",
            "type": "ping",
            "payload": [:],
        ])

        await fulfillment(of: [pongSent], timeout: 1)
        XCTAssertFalse(task.sentTexts.last?.hasSuffix("\n") ?? true)
        connection.disconnect()
    }

    @MainActor
    func testCancellingRequestDoesNotWaitForTransportTimeout() async throws {
        let task = ScriptedWebSocketTask()
        let profile = try ConnectionProfile.cowchatCloud(
            urlString: "wss://cloud.example/ws",
            apiKey: "cloud-key"
        )
        let connection = CowchatConnection(profile: profile) { _ in task }
        try await connection.connect()
        _ = try await connection.register(name: "Cowchat Mac", agentID: "mac-cancel")

        let requestSent = expectation(description: "list_agents request sent")
        task.onSend = { frame in
            if frame["type"] as? String == "list_agents" {
                requestSent.fulfill()
            }
        }
        let request = Task { try await connection.listAgents(roomID: "room") }
        await fulfillment(of: [requestSent], timeout: 1)

        request.cancel()
        do {
            _ = try await request.value
            XCTFail("Expected the cancelled request to finish immediately")
        } catch is CancellationError {
            // Expected: cancellation removes the pending continuation rather
            // than leaving callers blocked for the ten-second request timeout.
        }

        XCTAssertFalse(task.wasCancelled, "request cancellation must keep the transport alive")
        connection.disconnect()
    }

    @MainActor
    func testCloudRejectsOversizedOutgoingFrameBeforeTransportSend() async throws {
        let task = ScriptedWebSocketTask()
        let profile = try ConnectionProfile.cowchatCloud(
            urlString: "wss://cloud.example/ws",
            apiKey: "cloud-key"
        )
        let connection = CowchatConnection(profile: profile) { _ in task }
        try await connection.connect()
        _ = try await connection.register(name: "Cowchat Mac", agentID: "mac-large-send")

        do {
            _ = try await connection.send(
                roomID: "room",
                content: String(repeating: "x", count: 1_048_576)
            )
            XCTFail("Expected the oversized frame to be rejected")
        } catch let error as CowchatConnectionError {
            XCTAssertEqual(error.errorDescription, "Cowchat frames cannot exceed 1 MiB.")
        }
        XCTAssertEqual(task.sentFrames.count, 1, "Only registration should reach the socket")
        connection.disconnect()
    }

    @MainActor
    func testCloudClosesAfterOversizedIncomingFrame() async throws {
        let task = ScriptedWebSocketTask()
        let profile = try ConnectionProfile.cowchatCloud(
            urlString: "wss://cloud.example/ws",
            apiKey: "cloud-key"
        )
        let connection = CowchatConnection(profile: profile) { _ in task }
        try await connection.connect()
        _ = try await connection.register(name: "Cowchat Mac", agentID: "mac-large-receive")
        let failed = expectation(description: "oversized frame closes connection")
        connection.onStatusChange = { status in
            if case .failed(let message) = status,
               message == "Cowchat sent a frame larger than 1 MiB." {
                failed.fulfill()
            }
        }

        task.enqueue(text: String(repeating: "x", count: 1_048_577))

        await fulfillment(of: [failed], timeout: 1)
        XCTAssertTrue(task.wasCancelled)
    }

    @MainActor
    func testReconfigureCleanlyCancelsCloudTransport() async throws {
        let task = ScriptedWebSocketTask()
        let profile = try ConnectionProfile.cowchatCloud(
            urlString: "wss://cloud.example/ws",
            apiKey: "cloud-key"
        )
        let connection = CowchatConnection(profile: profile) { _ in task }
        try await connection.connect()

        connection.reconfigure(profile: .local)

        XCTAssertTrue(task.wasCancelled)
        XCTAssertEqual(connection.profile, .local)
    }
}

extension CowchatWebSocketTransportTests {
    @MainActor
    func testHistoryCollectsSizeBoundedPagesUsingSequenceCursors() async throws {
        let task = ScriptedWebSocketTask()
        let profile = try ConnectionProfile.cowchatCloud(urlString: "wss://cloud.example/ws", apiKey: "test-key")
        let connection = CowchatConnection(profile: profile) { _ in task }
        var historyRequests = 0
        task.onSend = { frame in
            guard frame["type"] as? String == "get_history" else { return }
            let request = frame["payload"] as! [String: Any]
            historyRequests += 1
            if historyRequests == 2 {
                XCTAssertEqual(request["before_seq"] as? Int, 3)
                XCTAssertEqual(request["limit"] as? Int, 2)
            }
            let seqs = historyRequests == 1 ? [3] : [1, 2]
            let messages = seqs.map { seq in
                ["message_id": "m\(seq)", "room_id": "room", "agent_id": "writer", "agent_name": "Writer", "content": "hello", "metadata": [:], "timestamp": "2026-10-16T00:00:00Z", "seq": seq] as [String: Any]
            }
            task.enqueue(frame: ["reply_to": frame["id"]!, "type": "history_result", "payload": ["messages": messages, "has_more": historyRequests == 1]])
        }
        try await connection.connect()
        _ = try await connection.register(name: "Reader", agentID: "reader")
        defer { connection.disconnect() }
        let messages = try await connection.history(roomID: "room", limit: 3)
        XCTAssertEqual(messages.map(\.seq), [1, 2, 3])
        XCTAssertEqual(historyRequests, 2)
    }
}
