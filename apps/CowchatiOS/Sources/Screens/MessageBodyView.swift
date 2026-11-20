import SwiftUI
import CowchatKit

struct MessageBodyView: View {
    let body_: TimelineEntry.Body

    var body: some View {
        switch body_ {
        case .locked:
            HStack(spacing: 6) {
                GallopIconView(icon: .lock, fallbackSystemName: "lock.fill", size: 16)
                Text("Encrypted message").gallopText(.bodyL)
            }
            .foregroundStyle(SemanticColor.textTertiary)
        case .chart(let title, let png):
            ChartView(title: title, png: png)
        case .text(let content):
            VStack(alignment: .leading, spacing: 8) {
                ForEach(MessageContentParser.segments(in: content)) { SegmentView(segment: $0) }
            }
            // Message text comes from other room members; only web links may leave the app.
            .environment(\.openURL, OpenURLAction { url in
                ["http", "https"].contains(url.scheme?.lowercased() ?? "") ? .systemAction : .discarded
            })
        }
    }
}

private struct SegmentView: View {
    let segment: MessageContentSegment

    var body: some View {
        switch segment.kind {
        case .code:
            ScrollView(.horizontal) {
                Text(segment.text)
                    .gallopText(.codeSm, color: SemanticColor.textPrimary)
                    .textSelection(.enabled)
                    .padding(10)
            }
            .background(SemanticColor.surface600, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
            .overlay {
                RoundedRectangle(cornerRadius: 8, style: .continuous).stroke(SemanticColor.borderDefault, lineWidth: 0.5)
            }
        case .table:
            // Columns line up only in a fixed-width face, and a wide table scrolls sideways.
            ScrollView(.horizontal) {
                Text(segment.text)
                    .gallopText(.codeSm, color: SemanticColor.textPrimary)
                    .textSelection(.enabled)
            }
        case .heading:
            Text(MessageContentParser.attributedInline(segment.text)).gallopText(.h5)
        case .blockQuote:
            Text(MessageContentParser.attributedInline(segment.text))
                .gallopText(.bodyL, color: SemanticColor.textTertiary)
                .padding(.leading, 10)
                .overlay(alignment: .leading) { Rectangle().fill(SemanticColor.borderDefault).frame(width: 2) }
        default:
            Text(MessageContentParser.attributedInline(segment.text)).gallopText(.bodyL)
        }
    }
}
