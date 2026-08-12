import Foundation

struct MessageContentSegment: Equatable, Identifiable {
    enum Kind: Equatable {
        case prose
        case heading(level: Int)
        case unorderedList
        case orderedList
        case blockQuote
        case table
        case code
    }

    let id: Int
    let kind: Kind
    let text: String
}

enum MessageContentParser {
    static func segments(in content: String) -> [MessageContentSegment] {
        let content = normalizedLineEndings(in: content)
        let lines = content.split(separator: "\n", omittingEmptySubsequences: false).map(String.init)
        var result: [MessageContentSegment] = []
        var proseLines: [String] = []
        var code = ""
        var openFence: Fence?
        var activeListIndent: Int?

        func append(_ kind: MessageContentSegment.Kind, _ text: String) {
            guard !text.isEmpty || kind == .code else { return }
            result.append(.init(id: result.count, kind: kind, text: text))
        }

        func flushProse() {
            for block in proseBlocks(in: proseLines.joined(separator: "\n")) {
                append(block.kind, block.text)
            }
            proseLines.removeAll(keepingCapacity: true)
        }

        for (index, line) in lines.enumerated() {
            if let fence = openFence {
                if isClosingFence(line, matching: fence) {
                    append(.code, code)
                    code = ""
                    openFence = nil
                } else {
                    code += codeLine(line, matching: fence)
                    if index < lines.count - 1 { code += "\n" }
                }
            } else if let fence = openingFence(
                in: line,
                indentLimit: activeListIndent.map { $0 + 3 } ?? 3
            ) {
                flushProse()
                openFence = fence
            } else {
                proseLines.append(line)
                if let listIndent = listContentIndent(in: line) {
                    activeListIndent = listIndent
                } else if line.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    // A blank line is allowed before a fenced child block.
                } else if let indent = activeListIndent,
                          leadingIndent(in: line) >= indent {
                    // Continue tracking the current list container.
                } else {
                    activeListIndent = nil
                }
            }
        }

        if openFence != nil {
            append(.code, code)
        } else {
            flushProse()
        }
        if result.isEmpty, content.isEmpty {
            return [.init(id: 0, kind: .prose, text: "")]
        }
        return result
    }

    static func attributedInline(_ source: String) -> AttributedString {
        var options = AttributedString.MarkdownParsingOptions()
        options.interpretedSyntax = .inlineOnlyPreservingWhitespace
        return (try? AttributedString(markdown: source, options: options))
            ?? AttributedString(source)
    }

    private struct ProseBlock {
        let kind: MessageContentSegment.Kind
        let text: String
    }

    private struct Fence {
        let marker: Character
        let length: Int
        let quoteDepth: Int
        let outerIndent: Int
        let contentIndent: Int
        let outerClosingIndentLimit: Int
        let contentClosingIndentLimit: Int
    }

    private struct FenceCandidate {
        let content: Substring
        let quoteDepth: Int
        let outerIndent: Int
        let contentIndent: Int
        let outerClosingIndentLimit: Int
        let contentClosingIndentLimit: Int
    }

    private static func proseBlocks(in source: String) -> [ProseBlock] {
        let lines = source.split(separator: "\n", omittingEmptySubsequences: false).map(String.init)
        var result: [ProseBlock] = []
        var bufferedKind: MessageContentSegment.Kind?
        var bufferedLines: [String] = []
        var index = 0

        func flush() {
            guard let kind = bufferedKind, !bufferedLines.isEmpty else { return }
            let text = kind == .prose
                ? joinedProseLines(bufferedLines)
                : bufferedLines.joined(separator: "\n")
            result.append(.init(kind: kind, text: text))
            bufferedKind = nil
            bufferedLines.removeAll(keepingCapacity: true)
        }

        func buffer(_ kind: MessageContentSegment.Kind, line: String) {
            if bufferedKind != kind {
                flush()
                bufferedKind = kind
            }
            bufferedLines.append(line)
        }

        while index < lines.count {
            let line = lines[index]
            if line.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                flush()
                index += 1
                continue
            }

            if index + 1 < lines.count,
               blockContent(in: line)?.contains("|") == true,
               isTableDelimiter(lines[index + 1]) {
                flush()
                var tableLines = [line, lines[index + 1]]
                index += 2
                while index < lines.count,
                      !lines[index].trimmingCharacters(in: .whitespaces).isEmpty,
                      blockContent(in: lines[index])?.contains("|") == true {
                    tableLines.append(lines[index])
                    index += 1
                }
                result.append(.init(kind: .table, text: tableLines.joined(separator: "\n")))
                continue
            }

            if let heading = heading(in: line) {
                flush()
                result.append(.init(kind: .heading(level: heading.level), text: heading.text))
            } else if let kind = bufferedKind,
                      (kind == .unorderedList || kind == .orderedList),
                      leadingIndent(in: line) > 0 {
                // A differently-marked child list still belongs to its
                // top-level parent. Check continuation before marker kind so
                // mixed nested lists remain one faithful block.
                buffer(kind, line: line)
            } else if isUnorderedListItem(line) {
                buffer(.unorderedList, line: line)
            } else if isOrderedListItem(line) {
                buffer(.orderedList, line: line)
            } else if let quote = blockQuoteText(in: line) {
                buffer(.blockQuote, line: quote)
            } else if let indentedCode = indentedCodeText(in: line) {
                buffer(.code, line: indentedCode)
            } else {
                buffer(.prose, line: line)
            }
            index += 1
        }

        flush()
        return result
    }

    private static func heading(in line: String) -> (level: Int, text: String)? {
        guard let content = blockContent(in: line) else { return nil }
        let marker = content.prefix { $0 == "#" }
        guard (1...6).contains(marker.count) else { return nil }
        let remainder = content.dropFirst(marker.count)
        guard remainder.first == " " || remainder.first == "\t" else { return nil }
        return (
            marker.count,
            String(remainder.dropFirst()).trimmingCharacters(in: .whitespaces)
        )
    }

    private static func isUnorderedListItem(_ line: String) -> Bool {
        guard let content = blockContent(in: line) else { return false }
        return content.hasPrefix("- ") || content.hasPrefix("* ") || content.hasPrefix("+ ")
    }

    private static func isOrderedListItem(_ line: String) -> Bool {
        guard let content = blockContent(in: line) else { return false }
        let digits = content.prefix { $0.isNumber }
        guard !digits.isEmpty else { return false }
        let remainder = content.dropFirst(digits.count)
        guard remainder.first == "." || remainder.first == ")" else { return false }
        return remainder.dropFirst().first == " "
    }

    private static func blockQuoteText(in line: String) -> String? {
        guard let content = blockContent(in: line), content.first == ">" else { return nil }
        return String(content.dropFirst()).trimmingCharacters(in: .whitespaces)
    }

    private static func isTableDelimiter(_ line: String) -> Bool {
        guard let content = blockContent(in: line) else { return false }
        let hasPipe = content.contains("|")
        let trimmed = String(content).trimmingCharacters(in: .whitespaces)
            .trimmingCharacters(in: CharacterSet(charactersIn: "|"))
        let cells = trimmed.split(separator: "|", omittingEmptySubsequences: false)
        // The header check already requires a pipe, so one-column tables such
        // as `| Status |` / `| --- |` cannot be mistaken for a thematic rule.
        guard !cells.isEmpty, cells.count > 1 || hasPipe else { return false }
        return cells.allSatisfy { cell in
            let candidate = String(cell).trimmingCharacters(in: .whitespaces)
            let dashes = candidate.trimmingCharacters(in: CharacterSet(charactersIn: ":"))
            return dashes.count >= 3 && dashes.allSatisfy { $0 == "-" }
        }
    }

    private static func normalizedLineEndings(in source: String) -> String {
        source.replacingOccurrences(of: "\r\n", with: "\n")
            .replacingOccurrences(of: "\r", with: "\n")
    }

    private static func openingFence(in line: String, indentLimit: Int) -> Fence? {
        guard let candidate = fenceCandidate(in: line, indentLimit: indentLimit),
              let marker = candidate.content.first,
              marker == "`" || marker == "~" else { return nil }
        let run = candidate.content.prefix { $0 == marker }
        guard run.count >= 3 else { return nil }
        let info = candidate.content.dropFirst(run.count)
        guard marker != "`" || !info.contains("`") else { return nil }
        return .init(
            marker: marker,
            length: run.count,
            quoteDepth: candidate.quoteDepth,
            outerIndent: candidate.outerIndent,
            contentIndent: candidate.contentIndent,
            outerClosingIndentLimit: candidate.outerClosingIndentLimit,
            contentClosingIndentLimit: candidate.contentClosingIndentLimit
        )
    }

    private static func isClosingFence(_ line: String, matching fence: Fence) -> Bool {
        guard let content = contentInsideContainer(line, matching: fence, closing: true) else {
            return false
        }
        let run = content.prefix { $0 == fence.marker }
        guard run.count >= fence.length else { return false }
        return content.dropFirst(run.count).allSatisfy { $0 == " " || $0 == "\t" }
    }

    private static func codeLine(_ line: String, matching fence: Fence) -> String {
        guard let content = contentInsideContainer(line, matching: fence, closing: false) else {
            // Malformed container input is still user content. Preserve it
            // instead of silently discarding a line while looking for a close.
            return line
        }
        return String(content)
    }

    /// Locates a fence after ordinary block indentation, blockquote markers,
    /// or a list marker. The stored container shape is used to strip only
    /// Markdown syntax from code lines; code indentation itself is preserved.
    private static func fenceCandidate(in line: String, indentLimit: Int) -> FenceCandidate? {
        var content = line[...]
        let initial = droppingSpaces(from: content, limit: indentLimit)
        content = initial.remainder

        var quoteDepth = 0
        while content.first == ">" {
            quoteDepth += 1
            content = content.dropFirst()
            if content.first == " " || content.first == "\t" {
                content = content.dropFirst()
            }
        }

        var contentIndent = initial.count
        var contentClosingIndentLimit = indentLimit
        if quoteDepth > 0 {
            let quoteContentIndent = droppingSpaces(from: content, limit: 3)
            content = quoteContentIndent.remainder
            contentIndent = quoteContentIndent.count
            contentClosingIndentLimit = 3
        }
        if let listContent = listItemContent(in: content) {
            content = listContent.remainder
            contentIndent += listContent.width
            contentClosingIndentLimit = max(
                contentClosingIndentLimit,
                contentIndent + 3
            )
        }

        return .init(
            content: content,
            quoteDepth: quoteDepth,
            outerIndent: initial.count,
            contentIndent: contentIndent,
            outerClosingIndentLimit: indentLimit,
            contentClosingIndentLimit: contentClosingIndentLimit
        )
    }

    private static func contentInsideContainer(
        _ line: String,
        matching fence: Fence,
        closing: Bool
    ) -> Substring? {
        var content = line[...]
        if fence.quoteDepth > 0 {
            let outerLimit = closing
                ? fence.outerClosingIndentLimit
                : fence.outerIndent
            content = droppingSpaces(from: content, limit: outerLimit).remainder
            for _ in 0..<fence.quoteDepth {
                guard content.first == ">" else { return nil }
                content = content.dropFirst()
                if content.first == " " || content.first == "\t" {
                    content = content.dropFirst()
                }
            }
            let contentLimit = closing
                ? fence.contentClosingIndentLimit
                : fence.contentIndent
            return droppingSpaces(from: content, limit: contentLimit).remainder
        }
        let contentLimit = closing
            ? fence.contentClosingIndentLimit
            : fence.contentIndent
        return droppingSpaces(from: content, limit: contentLimit).remainder
    }

    private static func droppingSpaces(
        from source: Substring,
        limit: Int
    ) -> (remainder: Substring, count: Int) {
        var remainder = source
        var count = 0
        while count < limit, remainder.first == " " {
            remainder = remainder.dropFirst()
            count += 1
        }
        return (remainder, count)
    }

    private static func listItemContent(
        in source: Substring
    ) -> (remainder: Substring, width: Int)? {
        if let marker = source.first,
           marker == "-" || marker == "*" || marker == "+",
           source.dropFirst().first == " " {
            return (source.dropFirst(2), 2)
        }

        let digits = source.prefix { $0.isNumber }
        guard !digits.isEmpty else { return nil }
        var remainder = source.dropFirst(digits.count)
        guard remainder.first == "." || remainder.first == ")" else { return nil }
        remainder = remainder.dropFirst()
        guard remainder.first == " " else { return nil }
        return (remainder.dropFirst(), digits.count + 2)
    }

    private static func listContentIndent(in line: String) -> Int? {
        let leading = leadingIndent(in: line)
        guard leading <= 3,
              let content = blockContent(in: line),
              let item = listItemContent(in: content) else { return nil }
        return leading + item.width
    }

    /// CommonMark block markers may be indented by at most three spaces.
    private static func blockContent(in line: String) -> Substring? {
        var index = line.startIndex
        var spaces = 0
        while index < line.endIndex, line[index] == " " {
            spaces += 1
            guard spaces <= 3 else { return nil }
            index = line.index(after: index)
        }
        return line[index...]
    }

    private static func leadingIndent(in line: String) -> Int {
        if line.first == "\t" { return 4 }
        return line.prefix { $0 == " " }.count
    }

    private static func indentedCodeText(in line: String) -> String? {
        if line.first == "\t" { return String(line.dropFirst()) }
        guard line.hasPrefix("    ") else { return nil }
        return String(line.dropFirst(4))
    }

    private static func joinedProseLines(_ lines: [String]) -> String {
        guard var result = lines.first else { return "" }
        for line in lines.dropFirst() {
            let preservesBreak = result.hasSuffix("  ") || result.hasSuffix("\\")
            result += preservesBreak ? "\n" : " "
            result += preservesBreak
                ? line
                : String(line.drop { $0 == " " || $0 == "\t" })
        }
        return result
    }
}
