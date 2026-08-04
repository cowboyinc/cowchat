import Foundation

enum MessagePreview {
    static let characterLimit = 1_200
    static let lineLimit = 5

    static func needsDisclosure(for content: String) -> Bool {
        content.count > characterLimit || content.filter(\.isNewline).count >= lineLimit
    }

    static func collapsedContent(for content: String) -> String {
        guard needsDisclosure(for: content) else { return content }

        var preview = content
        if content.count > characterLimit {
            preview = String(content.prefix(characterLimit))
        }

        let lines = preview.split(separator: "\n", omittingEmptySubsequences: false)
        if lines.count > lineLimit {
            preview = lines.prefix(lineLimit).joined(separator: "\n")
        }

        return preview + "…"
    }
}
