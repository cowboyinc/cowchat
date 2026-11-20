import Foundation
import CowchatKit

struct SignInForm: Equatable {
    enum Place: Equatable { case global, ownServer, otherServer }

    var place = Place.global
    var host = ""
    var port = "9229"
    var url = ServerChoice.defaultGlobalURL
    var pasted = ""

    var server: ServerChoice? {
        switch place {
        case .global:
            return .defaultGlobal
        case .ownServer:
            let host = host.trimmingCharacters(in: .whitespaces)
            guard !host.isEmpty, let port = UInt16(port) else { return nil }
            return .tcp(host: host, port: port)
        case .otherServer:
            let url = url.trimmingCharacters(in: .whitespaces)
            return url.hasPrefix("wss://") && url.count > 6 ? .global(url: url) : nil
        }
    }

    private var detected: PastedCredential { PastedCredential.detect(pasted) }
    var found: String? { detected.found }
    var credential: Credential? { detected.credential }
    var canConnect: Bool { server != nil && credential != nil }

    /// The server to offer provider sign-in against. Plain TCP servers have none.
    var providerURL: String? {
        guard case .global(let url) = server else { return nil }
        return url
    }
}

extension ServerChoice {
    static var defaultGlobalURL: String { ConnectionProfile.defaultGlobalURLString }
}
