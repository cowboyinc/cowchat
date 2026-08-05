import Foundation

struct MessageContentSegment: Equatable, Identifiable {
    enum Kind: Equatable {
        case prose
        case code
    }

    let id: Int
    let kind: Kind
    let text: String
}

enum MessageContentParser {
    static func segments(in content: String) -> [MessageContentSegment] {
        var remaining = content[...]
        var result: [MessageContentSegment] = []

        func append(_ kind: MessageContentSegment.Kind, _ text: Substring) {
            guard !text.isEmpty else { return }
            result.append(.init(id: result.count, kind: kind, text: String(text)))
        }

        while let opening = remaining.range(of: "```") {
            append(.prose, remaining[..<opening.lowerBound])
            var codeStart = opening.upperBound
            if let headerEnd = remaining[codeStart...].firstIndex(of: "\n") {
                codeStart = remaining.index(after: headerEnd)
            }

            let codeAndRemainder = remaining[codeStart...]
            if let closing = codeAndRemainder.range(of: "```") {
                append(.code, codeAndRemainder[..<closing.lowerBound])
                remaining = codeAndRemainder[closing.upperBound...]
            } else {
                append(.code, codeAndRemainder)
                remaining = remaining[remaining.endIndex...]
            }
        }

        append(.prose, remaining)
        if result.isEmpty, content.isEmpty {
            return [.init(id: 0, kind: .prose, text: "")]
        }
        return result
    }
}
