import Foundation

enum MessagePreview {
    static let characterLimit = 1_200
    static let lineLimit = 5

    struct CollapsedPreview: Equatable {
        let source: String
        let isTruncated: Bool
    }

    static func needsDisclosure(for content: String) -> Bool {
        let content = normalizedLineEndings(in: content)
        return content.count > characterLimit
            || content.lazy.filter { $0 == "\n" }.prefix(lineLimit).count == lineLimit
    }

    /// Returns an exact prefix for rich-text parsing and records truncation
    /// separately. Keeping the ellipsis out of the Markdown source prevents a
    /// cutoff next to a fence or inline delimiter from consuming it.
    static func collapsedPreview(for content: String) -> CollapsedPreview {
        guard needsDisclosure(for: content) else {
            return .init(source: content, isTruncated: false)
        }

        let normalized = normalizedLineEndings(in: content)
        var preview = normalized
        if normalized.count > characterLimit {
            preview = String(normalized.prefix(characterLimit))
        }

        let lines = preview.split(separator: "\n", omittingEmptySubsequences: false)
        if lines.count > lineLimit {
            preview = lines.prefix(lineLimit).joined(separator: "\n")
        }

        return .init(source: preview, isTruncated: preview != normalized)
    }

    private static func normalizedLineEndings(in source: String) -> String {
        source.replacingOccurrences(of: "\r\n", with: "\n")
            .replacingOccurrences(of: "\r", with: "\n")
    }
}
