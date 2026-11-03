import SwiftUI

/// The kinds of room New room makes (add-new-company note, section 3b, decided by Patrick on 2 November): account
/// rooms, rooms on this Mac's server, and rooms on the API-key connection. It never makes a session's room, since the
/// server makes those when a session registers.
enum NewRoomKind: String, CaseIterable, Identifiable {
    case account
    case thisMac
    case apiKey

    var id: String { rawValue }

    init?(server: WorkspaceStore.Server) {
        switch server {
        case .accountGlobal: self = .account
        case .local: self = .thisMac
        case .global: self = .apiKey
        }
    }

    var server: WorkspaceStore.Server {
        switch self {
        case .account: return .accountGlobal
        case .thisMac: return .local
        case .apiKey: return .global
        }
    }

    // copy:
    var title: String {
        switch self {
        case .account: return "Account"
        case .thisMac: return "This Mac"
        case .apiKey: return "API key"
        }
    }

    // copy:
    /// The error line while the kind's connection is not connected.
    var offlineLine: String {
        switch self {
        case .account: return "Your account isn't connected."
        case .thisMac: return "This Mac's server isn't running."
        case .apiKey: return "The API key's server isn't connected."
        }
    }

    /// The kinds the menu lists, in this order: Account while the person is signed in, This Mac always, and API key
    /// when the API-key connection is set up.
    static func offered(signedIn: Bool, hasAPIKey: Bool) -> [NewRoomKind] {
        (signedIn ? [.account] : []) + [.thisMac] + (hasAPIKey ? [.apiKey] : [])
    }

    /// The kind the sheet starts on when nothing names one: Account while signed in, This Mac otherwise. It does not
    /// follow the open room.
    static func starting(signedIn: Bool) -> NewRoomKind { signedIn ? .account : .thisMac }

    // copy:
    /// What is wrong with a typed name, as `CreateRoomView` checked it, or nil.
    static func nameProblem(_ name: String) -> String? {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        if trimmed.unicodeScalars.count > 100 { return "Room names can contain at most 100 characters." }
        if trimmed.unicodeScalars.contains(where: CharacterSet.controlCharacters.contains) {
            return "Room names cannot contain control characters."
        }
        return nil
    }
}

/// How New room opens: the kind it starts on and the account room's fields.
struct NewRoomRequest {
    let id = UUID()
    /// The kind the sheet starts on, or nil for `NewRoomKind.starting`.
    var kind: NewRoomKind?
    var draft = SocialRoomDraft()
}

/// What New room's Create came to.
enum NewRoomOutcome: Equatable {
    case opened
    case failed(String)
}

/// New room (add-new-company note, section 3b): one menu at the top chooses the kind, then the kind's fields. An
/// account room has Name and People, a room on this Mac has Name, and a room on the API key has Name and the server it
/// goes to. No room is made inside another, since the app shows no folders (navigation round 2, section 3.7). It replaces `RoomCreationSheet`, `CreateRoomView` and P6's `SocialRoomCreationView`, and keeps
/// their create logic in `WorkspaceStore.createRoom`.
struct NewRoomSheet: View {
    @ObservedObject var workspace: WorkspaceStore
    @ObservedObject var social: SocialStore
    let request: NewRoomRequest
    @Environment(\.dismiss) private var dismiss
    @State private var kind: NewRoomKind
    @State private var name = ""
    @State private var draft: SocialRoomDraft
    @State private var busy = false
    @State private var error: String?
    @FocusState private var isNameFocused: Bool

    init(workspace: WorkspaceStore, request: NewRoomRequest) {
        self.workspace = workspace
        social = workspace.social
        self.request = request
        let signedIn = workspace.social.phase == .signedIn
        _kind = State(initialValue: request.kind ?? NewRoomKind.starting(signedIn: signedIn))
        _draft = State(initialValue: request.draft)
    }

    private var offered: [NewRoomKind] {
        NewRoomKind.offered(signedIn: social.phase == .signedIn, hasAPIKey: workspace.global != nil)
    }

    /// The account room's People rows: the person's friends.
    private var people: [SocialAccount] { social.friends.map(\.account) }

    private var isConnected: Bool { workspace.store(for: kind.server)?.connectionStatus.isConnected == true }

    private var typedName: String { name.trimmingCharacters(in: .whitespacesAndNewlines) }

    private var canCreate: Bool {
        !typedName.isEmpty && NewRoomKind.nameProblem(name) == nil && isConnected && !busy
            && (kind != .account || social.phase == .signedIn)
    }

    private var errorLine: String? {
        if let problem = NewRoomKind.nameProblem(name) { return problem }
        if !isConnected { return kind.offlineLine }
        return error
    }

    var body: some View {
        // copy:
        SheetFrame(title: "New room", error: errorLine) {
            // copy:
            SheetGroup(label: "Where") { kindMenu }
            // copy:
            SheetGroup(label: "Name") {
                CapsuleField(placeholder: "Room name", text: $name, onSubmit: create)
                    .focused($isNameFocused)
            }
            if kind == .account {
                // copy:
                SheetGroup(label: "People") {
                    PeoplePicker(people: people, picked: $draft.friendIDs, placeholder: "Search friends",
                                 emptyLine: "No friends yet", submitEmpty: create)
                }
            }
            if kind == .apiKey {
                // copy:
                SheetGroup(label: "Server") {
                    Text(workspace.global.flatMap { ConnectionProfile.displayHost($0.connectionProfile) } ?? "")
                        .gallopText(.bodyM, color: SemanticColor.textSecondary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                        .frame(height: 20)
                }
            }
        } buttons: {
            // copy:
            SheetButtons(act: "Create", isEnabled: canCreate, busy: busy, cancel: { dismiss() }, perform: create)
        }
        .onAppear { isNameFocused = true }
        .onChange(of: name) { _ in error = nil }
        .onChange(of: kind) { _ in error = nil }
        .onChange(of: offered) { offered in
            if !offered.contains(kind), let first = offered.first { kind = first }
        }
    }

    private var kindMenu: some View {
        SheetMenuField(title: kind.title, accessibilityLabel: "Where") {
            ForEach(offered) { option in
                Toggle(option.title, isOn: Binding(get: { option == kind }, set: { if $0 { kind = option } }))
            }
        }
    }

    private func create() {
        guard canCreate else { return }
        busy = true
        error = nil
        Task {
            let outcome = await workspace.createRoom(kind, name: name, draft: draft)
            busy = false
            switch outcome {
            case .opened: dismiss()
            case let .failed(message): error = message
            }
        }
    }
}
