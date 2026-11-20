import Foundation
import CowchatKit

/// Pure mention bookkeeping: suggestions for a trailing `@query`, and the
/// ids of picked members whose `@name` still appears in the text.
struct MentionDraft: Equatable {
    private(set) var picked: [RoomMember] = []

    static func query(in text: String) -> String? {
        guard let at = text.lastIndex(of: "@") else { return nil }
        if at != text.startIndex, !text[text.index(before: at)].isWhitespace { return nil }
        let query = text[text.index(after: at)...]
        guard query.allSatisfy({ $0.isLetter || $0.isNumber || $0 == "_" || $0 == "." || $0 == "-" }) else {
            return nil
        }
        return String(query)
    }

    static func suggestions(text: String, members: [RoomMember]) -> [RoomMember] {
        guard let query = query(in: text) else { return [] }
        guard !query.isEmpty else { return members }
        return members.filter { $0.name.localizedCaseInsensitiveContains(query) }
    }

    /// Replaces the trailing `@query` with `@name ` and remembers the member.
    mutating func accept(_ member: RoomMember, in text: String) -> String {
        guard let at = text.lastIndex(of: "@"), Self.query(in: text) != nil else { return text }
        if !picked.contains(where: { $0.id == member.id }) { picked.append(member) }
        return String(text[..<at]) + "@\(member.name) "
    }

    func mentionIDs(in text: String) -> [String] {
        picked.filter { text.contains("@\($0.name)") }.map(\.id)
    }

    mutating func reset() { picked = [] }
}
