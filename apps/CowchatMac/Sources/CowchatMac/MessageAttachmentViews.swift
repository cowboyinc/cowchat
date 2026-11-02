import AppKit
import Quartz
import SwiftUI

/// A run of agent messages the room hides, as one quiet row lined up with the messages' text. It is the feed's "Show
/// full response" control with other words: Caption in Text/Tertiary, 6 between the glyph, the words and the fold's
/// chevron, here with Gallop's chatbot glyph for agents. The words rise to Text/Secondary under the pointer. It opens
/// the run in place, its messages following as quiet rows, and closes it again.
struct HiddenRunRow: View {
    let run: HiddenMessageRun
    let toggle: () -> Void
    @State private var isHovering = false

    /// The messages' text starts after the 24 avatar and the 11 beside it.
    static let indent: CGFloat = 35

    var body: some View {
        Button(action: toggle) {
            HStack(spacing: 6) {
                GallopIconView(icon: .chatbot, fallbackSystemName: "bubble.left", size: 12)
                Text(run.title)
                    .gallopText(.caption)
                    .lineLimit(1)
                GallopIconView(
                    icon: run.isOpen ? .chevronDownExtraSmall : .chevronRightExtraSmall,
                    fallbackSystemName: run.isOpen ? "chevron.down" : "chevron.right",
                    size: 10
                )
            }
            .foregroundStyle(isHovering ? SemanticColor.textSecondary : SemanticColor.textTertiary)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { isHovering = $0 }
        .macAccessibleAction(label: run.title, value: run.isOpen ? "expanded" : "collapsed", action: toggle)
        .padding(.leading, Self.indent)
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// A pull request a message carries, as one row under its text, ported from the Cowchat demo's row: GitHub's pull
/// request mark, the repository's name and number in Body M, the title in Body M Text/Secondary, then the checks and
/// the diff size. Passing checks draw Gallop's success mark in Notification/Success/Icon, failing ones its circled
/// cross in Notification/Error/Icon, and running ones the sidebar's working mark; lines added take
/// Notification/Success/Text and lines removed Notification/Error/Text, in Body S. It hugs its content up to the
/// column, 40 tall, with a 1-point Border/Default edge and corners of 12, taking Button/Secondary/Hover and
/// Border/Hover under the pointer. When it does not fit on one line the title wraps beside the name and the checks and
/// diff move under it. Pressing it opens the address through `MessageLinks`, and the hover shows the address.
struct PullRequestRow: View {
    let pullRequest: MessagePullRequest
    @State private var isHovering = false

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: 12, style: .continuous)
        Button(action: open) {
            ViewThatFits(in: .horizontal) {
                oneLine
                twoLines
            }
            .background(isHovering ? SemanticColor.buttonSecondaryHover : .clear, in: shape)
            .overlay {
                shape.strokeBorder(isHovering ? SemanticColor.borderHover : SemanticColor.borderDefault, lineWidth: 1)
            }
            .contentShape(shape)
        }
        .buttonStyle(.plain)
        .help(pullRequest.address)
        .onHover { isHovering = $0 }
        // copy:
        .macAccessibleAction(label: "Open pull request \(pullRequest.name)", value: accessibilityValue, action: open)
    }

    private func open() {
        if let url = pullRequest.url { MessageLinks.open(url) }
    }

    private var accessibilityValue: String {
        [pullRequest.title, pullRequest.checksLabel].compactMap { $0 }.joined(separator: ", ")
    }

    private var oneLine: some View {
        HStack(spacing: 8) {
            mark
            name
            Text(pullRequest.title)
                .gallopText(.bodyM, color: SemanticColor.textSecondary)
                .lineLimit(1)
            if hasStatus {
                status
                    .padding(.leading, 4)
            }
        }
        .padding(.horizontal, 12)
        .frame(height: 40)
    }

    private var twoLines: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                mark
                    .alignmentGuide(.firstTextBaseline) { $0[VerticalAlignment.center] + 4 }
                name
                Text(pullRequest.title)
                    .gallopText(.bodyM, color: SemanticColor.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if hasStatus {
                status
                    .padding(.leading, 24)
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 10)
    }

    private var hasStatus: Bool { pullRequest.checks != nil || pullRequest.diff != nil }

    private var status: some View {
        HStack(spacing: 8) {
            checks
            if let diff = pullRequest.diff {
                HStack(spacing: 4) {
                    Text(diff.added)
                        .gallopText(.bodyS, color: SemanticColor.notificationSuccessTextDefault)
                    Text(diff.removed)
                        .gallopText(.bodyS, color: SemanticColor.notificationErrorTextDefault)
                }
                .lineLimit(1)
                .fixedSize()
                .monospacedDigit()
            }
        }
    }

    private var mark: some View {
        PullRequestMark()
            .stroke(SemanticColor.iconSecondary, style: StrokeStyle(lineWidth: 1.5, lineCap: .round, lineJoin: .round))
            .frame(width: 16, height: 16)
    }

    private var name: some View {
        Text(pullRequest.name)
            .gallopText(.bodyM, color: SemanticColor.textPrimary)
            .lineLimit(1)
            .fixedSize()
    }

    @ViewBuilder private var checks: some View {
        if let label = pullRequest.checksLabel {
            Group {
                switch pullRequest.checks {
                case .passing:
                    GallopIconView(icon: .success, fallbackSystemName: "checkmark.circle.fill", size: 16)
                        .foregroundStyle(SemanticColor.notificationSuccessIconPrimary)
                case .failing:
                    GallopIconView(icon: .circleX, fallbackSystemName: "xmark.circle.fill", size: 16)
                        .foregroundStyle(SemanticColor.notificationErrorIconPrimary)
                default:
                    SessionWorkingMark()
                        .frame(width: 16, height: 16)
                }
            }
            .help(label)
        }
    }
}

/// GitHub's pull request mark on a 16 grid, stroked by the caller: two rings on the left joined by a line, and a ring on
/// the right whose line rises and turns left into an arrow. Gallop has no pull request icon, so the row draws its own,
/// as the demo's does.
struct PullRequestMark: Shape {
    func path(in rect: CGRect) -> Path {
        let unit = min(rect.width, rect.height) / 16
        func point(_ x: CGFloat, _ y: CGFloat) -> CGPoint { CGPoint(x: rect.minX + x * unit, y: rect.minY + y * unit) }
        let radius = 1.75 * unit
        var path = Path()
        for centre in [point(4, 3.5), point(4, 12.5), point(12, 12.5)] {
            path.addEllipse(in: CGRect(x: centre.x - radius, y: centre.y - radius, width: radius * 2, height: radius * 2))
        }
        path.move(to: point(4, 5.25))
        path.addLine(to: point(4, 10.75))
        path.move(to: point(12, 10.75))
        path.addLine(to: point(12, 6.5))
        path.addQuadCurve(to: point(9.5, 4), control: point(12, 4))
        path.addLine(to: point(7.5, 4))
        path.move(to: point(9.25, 2.25))
        path.addLine(to: point(7.5, 4))
        path.addLine(to: point(9.25, 5.75))
        return path
    }
}

/// A picture a message carries, as a thumbnail under its text: 120 tall at the picture's proportions, corners of 12
/// and the attachment chip's 1-point Border/Default edge, Border/Hover under the pointer. It loads over the attachment
/// path into memory, showing Surface/400 until it arrives. Pressing it saves the file and opens it in Quick Look.
struct AttachmentThumbnail: View {
    @EnvironmentObject private var store: ChatStore
    let attachment: FileAttachment
    let onOpen: (FileAttachment) -> Void
    @State private var image: NSImage?
    @State private var isHovering = false

    static let height: CGFloat = 120

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: 12, style: .continuous)
        Button { onOpen(attachment) } label: {
            Group {
                if let image {
                    Image(nsImage: image)
                        .resizable()
                        .interpolation(.high)
                } else {
                    SemanticColor.surface400
                }
            }
            .frame(width: width, height: Self.height)
            .clipShape(shape)
            .overlay {
                shape.strokeBorder(isHovering ? SemanticColor.borderHover : SemanticColor.borderDefault, lineWidth: 1)
            }
            .contentShape(shape)
        }
        .buttonStyle(.plain)
        .help(attachment.name)
        .onHover { isHovering = $0 }
        // copy:
        .macAccessibleAction(label: "Open \(attachment.name)") { onOpen(attachment) }
        .task(id: attachment.blobID) { image = await store.thumbnailImage(attachment) }
    }

    /// The picture's width at the thumbnail's height, or a 4:3 frame while it loads.
    private var width: CGFloat {
        guard let size = image?.size, size.height > 0 else { return Self.height * 4 / 3 }
        return (Self.height * size.width / size.height).rounded()
    }
}

/// Lays its children in rows that wrap at the proposed width, as the demo lays a message's thumbnails.
struct MessageFlowLayout: Layout {
    var spacing: CGFloat
    var lineSpacing: CGFloat

    private typealias Row = (items: [(index: Int, size: CGSize)], width: CGFloat, height: CGFloat)

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let rows = arrange(subviews, width: proposal.width ?? .infinity)
        let width = rows.map(\.width).max() ?? 0
        let height = rows.map(\.height).reduce(0, +) + lineSpacing * CGFloat(max(rows.count - 1, 0))
        return CGSize(width: width, height: height)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        var y = bounds.minY
        for row in arrange(subviews, width: bounds.width) {
            var x = bounds.minX
            for item in row.items {
                subviews[item.index].place(at: CGPoint(x: x, y: y + (row.height - item.size.height) / 2),
                                           proposal: ProposedViewSize(item.size))
                x += item.size.width + spacing
            }
            y += row.height + lineSpacing
        }
    }

    private func arrange(_ subviews: Subviews, width: CGFloat) -> [Row] {
        var rows: [Row] = []
        for index in subviews.indices {
            let size = subviews[index].sizeThatFits(.unspecified)
            if let last = rows.last, last.width + spacing + size.width <= width {
                rows[rows.count - 1].items.append((index, size))
                rows[rows.count - 1].width += spacing + size.width
                rows[rows.count - 1].height = max(last.height, size.height)
            } else {
                rows.append(([(index, size)], size.width, size.height))
            }
        }
        return rows
    }
}

/// Shows a saved file in Quick Look's panel, as the space bar does in Finder. The panel finds its controller through
/// the responder chain, so this joins the chain behind the window while it shows a file.
final class QuickLookPanel: NSResponder, QLPreviewPanelDataSource {
    @MainActor static let shared = QuickLookPanel()
    private var url: URL?

    @MainActor func show(_ url: URL) {
        self.url = url
        guard let panel = QLPreviewPanel.shared() else { return }
        joinResponderChain()
        if panel.isVisible {
            panel.reloadData()
        } else {
            panel.makeKeyAndOrderFront(nil)
        }
    }

    @MainActor private func joinResponderChain() {
        guard let window = NSApp.keyWindow ?? NSApp.mainWindow else { return }
        var next = window.nextResponder
        while let responder = next {
            if responder === self { return }
            next = responder.nextResponder
        }
        nextResponder = window.nextResponder
        window.nextResponder = self
    }

    override func acceptsPreviewPanelControl(_ panel: QLPreviewPanel!) -> Bool { true }

    override func beginPreviewPanelControl(_ panel: QLPreviewPanel!) {
        panel.dataSource = self
    }

    override func endPreviewPanelControl(_ panel: QLPreviewPanel!) {
        panel.dataSource = nil
    }

    func numberOfPreviewItems(in panel: QLPreviewPanel!) -> Int { url == nil ? 0 : 1 }

    func previewPanel(_ panel: QLPreviewPanel!, previewItemAt index: Int) -> QLPreviewItem! {
        url as NSURL?
    }
}
