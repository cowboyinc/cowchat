import Foundation
import Network

@MainActor
final class SocialOAuthCallback {
    private var listener: NWListener?
    private var ready: CheckedContinuation<UInt16, Error>?
    private var completion: CheckedContinuation<String, Error>?
    private var completionCode: String?
    private var terminalError: Error?
    private var expectedRequestID: String?
    private var port: UInt16?
    private var timeout: Task<Void, Never>?
    private var connections: [NWConnection] = []

    func start() async throws -> UInt16 {
        let parameters = NWParameters.tcp
        parameters.requiredLocalEndpoint = .hostPort(host: "127.0.0.1", port: .any)
        let listener = try NWListener(using: parameters, on: .any)
        self.listener = listener
        listener.newConnectionHandler = { [weak self] connection in
            Task { @MainActor in
                guard let self, self.terminalError == nil, self.completionCode == nil else { connection.cancel(); return }
                self.connections.append(connection)
                connection.start(queue: .main)
                self.receive(connection, accumulated: Data())
            }
        }
        listener.stateUpdateHandler = { [weak self] state in
            Task { @MainActor in
                guard let self else { return }
                switch state {
                case .ready:
                    guard let port = self.listener?.port?.rawValue else { self.cancel(); return }
                    self.port = port
                    self.ready?.resume(returning: port)
                    self.ready = nil
                case .failed(let error): self.cancel(error)
                default: break
                }
            }
        }
        return try await withCheckedThrowingContinuation { continuation in
            ready = continuation
            listener.start(queue: .main)
        }
    }

    func wait(requestID: String, expiresIn: Int) async throws -> String {
        expectedRequestID = requestID
        timeout = Task { [weak self] in
            do { try await Task.sleep(nanoseconds: UInt64(min(max(expiresIn, 1), 600)) * 1_000_000_000) }
            catch { return }
            self?.cancel(CowchatConnectionError.transport("Sign-in expired. Please try again."))
        }
        if let terminalError { throw terminalError }
        if let completionCode { return completionCode }
        return try await withCheckedThrowingContinuation { completion = $0 }
    }

    func expect(requestID: String) { expectedRequestID = requestID }

    func cancel(_ error: Error = CancellationError()) {
        terminalError = error
        ready?.resume(throwing: error)
        ready = nil
        completion?.resume(throwing: error)
        completion = nil
        close()
    }

    private func close() {
        timeout?.cancel()
        timeout = nil
        listener?.cancel()
        listener = nil
        connections.forEach { $0.cancel() }
        connections = []
    }

    static func completionCode(from header: String, port: UInt16, requestID: String) -> String? {
        let lines = header.components(separatedBy: "\r\n")
        guard let first = lines.first else { return nil }
        let request = first.split(separator: " ")
        guard request.count == 3, request[0] == "GET", request[2] == "HTTP/1.1",
              request[1].hasPrefix("/"), !request[1].contains("#") else { return nil }
        let hosts = lines.dropFirst().filter { $0.lowercased().hasPrefix("host:") }
        guard hosts.count == 1, hosts[0].dropFirst(5).trimmingCharacters(in: .whitespaces) == "127.0.0.1:\(port)",
              let url = URLComponents(string: "http://127.0.0.1:\(port)\(request[1])"),
              url.path == "/cowchat/oauth", let items = url.queryItems, items.count == 2,
              items.filter({ $0.name == "request_id" }).count == 1,
              items.first(where: { $0.name == "request_id" })?.value == requestID,
              items.filter({ $0.name == "completion_code" }).count == 1,
              let code = items.first(where: { $0.name == "completion_code" })?.value,
              (32...512).contains(code.count), code.range(of: "^[A-Za-z0-9_-]+$", options: .regularExpression) != nil else { return nil }
        return code
    }

    private func receive(_ connection: NWConnection, accumulated: Data) {
        connection.receive(minimumIncompleteLength: 1, maximumLength: 8192) { [weak self] data, _, complete, error in
            Task { @MainActor in
                guard let self else { connection.cancel(); return }
                var header = accumulated
                if let data { header.append(data) }
                guard header.count <= 8192, error == nil else { connection.cancel(); return }
                guard let text = String(data: header, encoding: .utf8), text.contains("\r\n\r\n") else {
                    if complete { connection.cancel() } else { self.receive(connection, accumulated: header) }
                    return
                }
                guard let port = self.port, let requestID = self.expectedRequestID,
                      let code = Self.completionCode(from: text, port: port, requestID: requestID), self.completionCode == nil else {
                    self.respond(connection, status: "400 Bad Request", body: "Invalid Cowchat sign-in callback.")
                    return
                }
                self.completionCode = code
                self.respond(connection, status: "200 OK", body: "Cowchat sign-in received. You may close this browser tab.")
                self.completion?.resume(returning: code)
                self.completion = nil
                self.listener?.cancel()
                self.listener = nil
                self.timeout?.cancel()
                self.timeout = nil
            }
        }
    }

    private func respond(_ connection: NWConnection, status: String, body: String) {
        let response = "HTTP/1.1 \(status)\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: \(body.utf8.count)\r\nCache-Control: no-store\r\nContent-Security-Policy: default-src 'none'\r\nConnection: close\r\n\r\n\(body)"
        connection.send(content: Data(response.utf8), completion: .contentProcessed { _ in connection.cancel() })
    }
}
