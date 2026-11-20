import AppKit
import SwiftUI
import CowchatKit

/// The @names a room's messages draw in the accent, as the approved design draws them: the live sessions of the room's
/// owner, and the people the app knows, by handle or by first name ("@Patrick-Mandia" or "@Patrick"). A mention of the
/// viewer also sits on a fill. Names are kept lowercased, without their @.
struct MentionMarks: Equatable {
    /// One @name to draw in the accent.
    struct Mark: Equatable {
        let range: Range<String.Index>
        let namesViewer: Bool
    }

    private(set) var names: Set<String> = []
    private(set) var viewerNames: Set<String> = []

    init() {}

    init(sessionNames: [String], people: [SocialAccount], viewer: SocialAccount?) {
        viewerNames = Set(viewer.map(Self.names(of:)) ?? [])
        names = Set(sessionNames.map { $0.lowercased() } + people.flatMap(Self.names(of:))).union(viewerNames)
    }

    /// The @names in a text that name a session or person here. With `completedOnly`, a name that runs to the end of
    /// the text, which is still being typed, is left out.
    func marks(in text: String, completedOnly: Bool = false) -> [Mark] {
        guard !names.isEmpty else { return [] }
        return LiveSessions.mentionRanges(in: text).compactMap { range in
            let name = text[text.index(after: range.lowerBound)..<range.upperBound].lowercased()
            guard names.contains(name), !completedOnly || range.upperBound < text.endIndex else { return nil }
            return Mark(range: range, namesViewer: viewerNames.contains(name))
        }
    }

    /// A message's text with its @names in the accent, and a mention of the viewer on its fill.
    func marking(_ attributed: AttributedString) -> AttributedString {
        let text = String(attributed.characters)
        let found = marks(in: text)
        guard !found.isEmpty else { return attributed }
        var result = attributed
        for mark in found {
            let start = result.characters.index(result.startIndex,
                                                offsetBy: text.distance(from: text.startIndex, to: mark.range.lowerBound))
            let end = result.characters.index(start, offsetBy: text.distance(from: mark.range.lowerBound,
                                                                             to: mark.range.upperBound))
            result[start..<end].swiftUI.foregroundColor = SemanticColor.surfaceGlassOnTextDefault
            if mark.namesViewer { result[start..<end].swiftUI.backgroundColor = Self.viewerFill }
        }
        return result
    }

    /// The fill behind a mention of the viewer: Nugget 100 in light mode and Nugget 900 in dark, as the approved design
    /// draws it. The accent keeps 4.5:1 on either.
    static let viewerFill = Color(nsColor: NSColor(name: nil) { appearance in
        HexColor.isDark(appearance) ? NSColor(Palette.nugget900) : NSColor(Palette.nugget100)
    })

    /// The accent, Surface Glass On Text Default, as AppKit draws it in the composer.
    static let accent = NSColor(name: nil) { appearance in
        guard let token = SemanticColor.allTokens.first(where: { $0.name == "surfaceGlassOnTextDefault" }) else {
            return .controlAccentColor
        }
        return HexColor.nsColor(HexColor.isDark(appearance) ? token.dark : token.light)
    }

    /// A person's handle and first name, as others write them after @.
    private static func names(of account: SocialAccount) -> [String] {
        [account.handle, account.displayName.split(separator: " ").first.map(String.init) ?? ""]
            .filter { !$0.isEmpty }
            .map { $0.lowercased() }
    }
}

private struct MentionMarksKey: EnvironmentKey {
    static let defaultValue = MentionMarks()
}

extension EnvironmentValues {
    /// The @names the messages drawn here mark in the accent.
    var mentionMarks: MentionMarks {
        get { self[MentionMarksKey.self] }
        set { self[MentionMarksKey.self] = newValue }
    }
}
