import Foundation

/// The archive rule for ended sessions (Patrick, 2 November; navigation note, section 4.4). When a session ends, its
/// room moves to Archived, unless someone other than the person is in it or it has unread messages. A room with unread
/// messages archives once it is read, and the room the person has open archives once they open something else.
///
/// It runs in the app, because archive and read state are kept on each Mac (`ChatStore.archivedRoomIDs`,
/// `RoomReadState`). The server only says that a session ended, with `ended_at` on its room and a `session_state` of
/// `ended`. These are the rule's decisions as plain functions; `WorkspaceStore` asks the server for each room's
/// participants and applies them, and `RoomLocalPreferences` keeps what it did per room, so each ended session is
/// handled once.
enum SessionRoomArchive {
    /// What the rule did with one session room.
    enum Outcome: String, Codable {
        /// The rule archived it. The person may unarchive it, and it then stays in the list.
        case archived
        /// It was unread or open when its session ended, and archives once it is read and something else is open.
        case waiting
        /// Someone other than the person is in it, so it stays in the list and is not checked again.
        case kept
    }

    enum Decision: Equatable {
        case archive
        case wait
        case keep
    }

    /// The person's own session rooms the rule looks at now: those whose session has ended and that it has not handled
    /// yet, and those waiting to be read. A teammate's session room is never one of them, since the teammate is someone
    /// other than the person in it.
    static func roomsToCheck(_ rooms: [SocialRoom], viewerID: String, hasEnded: (SocialRoom.Session) -> Bool,
                             outcomes: [String: Outcome]) -> [SocialRoom] {
        rooms.filter { room in
            guard room.ownerId == viewerID, let session = room.session, hasEnded(session) else { return false }
            switch outcomes[room.roomId] {
            case nil, .waiting?: return true
            case .archived?, .kept?: return false
            }
        }
    }

    /// The rooms whose session is live again while the rule holds an entry for them. Their entries go, and a room the
    /// rule archived comes back to the list.
    static func resumed(_ rooms: [SocialRoom], viewerID: String, hasEnded: (SocialRoom.Session) -> Bool,
                        outcomes: [String: Outcome]) -> [String] {
        rooms.filter { room in
            guard room.ownerId == viewerID, let session = room.session else { return false }
            return outcomes[room.roomId] != nil && !hasEnded(session)
        }
        .map(\.roomId)
    }

    /// Whether someone other than the person is in the room: a member who is not them, or an invitation they sent for
    /// it that is still open. Agents don't count.
    static func hasSomeoneElse(members: [SocialAccount], viewerID: String, roomID: String,
                               sentInvitations: [SocialRoomInvitation]) -> Bool {
        members.contains { $0.id != viewerID }
            || sentInvitations.contains { $0.roomId == roomID && $0.status == "pending" }
    }

    /// What the rule does with a room whose session has ended, once it knows who is in it. A room the person has open
    /// counts as read, but waits until they open something else, so its cell does not vanish while they read it.
    static func decide(hasSomeoneElse: Bool, isUnread: Bool, isOpen: Bool) -> Decision {
        if hasSomeoneElse { return .keep }
        if isUnread || isOpen { return .wait }
        return .archive
    }

    /// Whether a waiting room is worth asking about again: only once it is read and closed, since until then it still
    /// waits whoever is in it.
    static func isReady(isUnread: Bool, isOpen: Bool) -> Bool {
        !isUnread && !isOpen
    }
}
