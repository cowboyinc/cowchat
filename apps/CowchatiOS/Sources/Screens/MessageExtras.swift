import ImageIO
import QuickLook
import SwiftUI
import CowchatKit

/// Pictures come from other room members, so they are never decoded at full size: the bytes and the pixel count are
/// capped, and ImageIO builds a small thumbnail straight from the file.
enum SafeImage {
    static let maxBytes = 20 * 1024 * 1024
    static let maxPixels = 40_000_000

    static func thumbnail(from data: Data, maxPixelSize: Int) -> UIImage? {
        guard data.count <= maxBytes,
              let source = CGImageSourceCreateWithData(data as CFData, [kCGImageSourceShouldCache: false] as CFDictionary),
              let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
              let width = properties[kCGImagePropertyPixelWidth] as? Int,
              let height = properties[kCGImagePropertyPixelHeight] as? Int,
              width > 0, height > 0, width <= maxPixels / height,
              let image = CGImageSourceCreateThumbnailAtIndex(source, 0, [
                  kCGImageSourceCreateThumbnailFromImageAlways: true,
                  kCGImageSourceCreateThumbnailWithTransform: true,
                  kCGImageSourceThumbnailMaxPixelSize: maxPixelSize,
                  kCGImageSourceShouldCacheImmediately: true,
              ] as CFDictionary)
        else { return nil }
        return UIImage(cgImage: image)
    }
}

/// A pull request a message carries: repository and number, title, checks and diff size. Opens in the browser.
struct PullRequestRow: View {
    let pullRequest: MessagePullRequest
    @Environment(\.openURL) private var openURL

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: 12, style: .continuous)
        Button {
            if let url = pullRequest.url { openURL(url) }
        } label: {
            VStack(alignment: .leading, spacing: 4) {
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Text(pullRequest.name).gallopText(.bodyMStrong, color: SemanticColor.textPrimary)
                    Text(pullRequest.title)
                        .gallopText(.bodyM, color: SemanticColor.textSecondary)
                        .multilineTextAlignment(.leading)
                }
                if pullRequest.checks != nil || pullRequest.diff != nil {
                    HStack(spacing: 8) {
                        if let label = pullRequest.checksLabel {
                            Text(label).gallopText(.bodyS, color: checksColor)
                        }
                        if let diff = pullRequest.diff {
                            Text(diff.added).gallopText(.bodyS, color: SemanticColor.notificationSuccessTextDefault)
                            Text(diff.removed).gallopText(.bodyS, color: SemanticColor.textError)
                        }
                    }
                }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 10)
            .frame(maxWidth: .infinity, alignment: .leading)
            .overlay { shape.strokeBorder(SemanticColor.borderDefault, lineWidth: 1) }
            .contentShape(shape)
        }
        .buttonStyle(.plain)
        .accessibilityIdentifier("message.pullRequest")
    }

    private var checksColor: Color {
        switch pullRequest.checks {
        case .passing: SemanticColor.notificationSuccessTextDefault
        case .failing: SemanticColor.textError
        case .running, nil: SemanticColor.textTertiary
        }
    }
}

/// The files under a message. Pictures draw as thumbnails; pressing any file opens it in Quick Look.
struct AttachmentsView: View {
    @EnvironmentObject private var session: ChatSession
    let attachments: [FileAttachment]
    let roomID: String

    @State private var preview: URL?
    @State private var errorMessage: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            ForEach(attachments, id: \.blobID) { file in
                let available = session.attachmentRequest(file, roomID: roomID) != nil
                Button { open(file) } label: {
                    if file.isImage && available {
                        AttachmentThumbnail(file: file, roomID: roomID)
                    } else {
                        HStack(spacing: 8) {
                            Image(systemName: "doc").foregroundStyle(SemanticColor.iconSecondary)
                            Text(file.name).gallopText(.bodyM, color: SemanticColor.textPrimary).lineLimit(1)
                            if let size = file.size {
                                Text(ByteCountFormatter.string(fromByteCount: Int64(size), countStyle: .file))
                                    .gallopText(.caption, color: SemanticColor.textTertiary)
                            }
                        }
                        .padding(.horizontal, 12)
                        .padding(.vertical, 8)
                        .overlay {
                            RoundedRectangle(cornerRadius: 12, style: .continuous)
                                .strokeBorder(SemanticColor.borderDefault, lineWidth: 1)
                        }
                    }
                }
                .buttonStyle(.plain)
                .disabled(!available)
                .accessibilityIdentifier("message.attachment")
            }
        }
        .quickLookPreview($preview)
        .alert(
            errorMessage ?? "",
            isPresented: Binding(get: { errorMessage != nil }, set: { if !$0 { errorMessage = nil } })
        ) {
            Button("OK", role: .cancel) {}
        }
    }

    private func open(_ file: FileAttachment) {
        Task {
            do {
                let data = try await session.attachmentData(file, roomID: roomID)
                let folder = FileManager.default.temporaryDirectory.appendingPathComponent(file.blobID, isDirectory: true)
                try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
                // The name comes from another room member, so only its last path part is used.
                let name = (file.name as NSString).lastPathComponent
                let url = folder.appendingPathComponent(name.isEmpty ? "attachment" : name)
                try data.write(to: url, options: .atomic)
                preview = url
            } catch {
                errorMessage = error.localizedDescription
            }
        }
    }
}

private struct AttachmentThumbnail: View {
    @EnvironmentObject private var session: ChatSession
    let file: FileAttachment
    let roomID: String
    @State private var image: UIImage?

    var body: some View {
        Group {
            if let image {
                Image(uiImage: image).resizable().scaledToFit()
            } else {
                SemanticColor.surface600
            }
        }
        .frame(maxWidth: 240, maxHeight: 180)
        .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
        .task(id: file.blobID) {
            guard image == nil, (file.size ?? 0) <= SafeImage.maxBytes,
                  let data = try? await session.attachmentData(file, roomID: roomID) else { return }
            image = SafeImage.thumbnail(from: data, maxPixelSize: 720)
        }
    }
}

struct ChartView: View {
    let title: String
    let png: Data

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(title).gallopText(.bodyMStrong, color: SemanticColor.textPrimary)
            if let image = SafeImage.thumbnail(from: png, maxPixelSize: 2048) {
                Image(uiImage: image)
                    .resizable()
                    .scaledToFit()
                    .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
                    .accessibilityLabel(title)
            } else {
                Text("This chart could not be displayed.").gallopText(.bodyS, color: SemanticColor.textTertiary)
            }
        }
        .accessibilityIdentifier("message.chart")
    }
}
