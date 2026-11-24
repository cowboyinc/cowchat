import AppKit
import Foundation
import CowchatKit

/// Where the app sends a message's web address, such as a pull request's. The prototype replaces it, so a press there
/// opens nothing.
@MainActor
enum MessageLinks {
    static var open: (URL) -> Void = { NSWorkspace.shared.open($0) }
}

/// Where a saved attachment opens when its thumbnail is pressed. Tests replace it.
@MainActor
enum AttachmentPreview {
    static var show: (URL) -> Void = { QuickLookPanel.shared.show($0) }
}

extension ChatStore {
    /// The session thumbnails load over. The prototype points it at its simulated server, as it does the download's.
    static var thumbnailSession: URLSession = .shared
    /// Pictures already loaded, by blob ID. Blobs never change once uploaded.
    private static var thumbnailImages: [String: NSImage] = [:]

    /// A picture attachment for its thumbnail, fetched as a download is but kept in memory, not saved. Nil when the
    /// room's server has no blob route or the blob is not a picture.
    func thumbnailImage(_ attachment: FileAttachment) async -> NSImage? {
        if let image = Self.thumbnailImages[attachment.blobID] { return image }
        guard selectedRoom?.encrypted != true, connectionProfile.kind == .cowchatCloud,
              let url = WorkspaceStore.blobURL(
                  forCloudURLString: connectionProfile.endpointDescription,
                  blobID: attachment.blobID
              )
        else { return nil }
        var request = URLRequest(url: url)
        request.setValue(connectionProfile.apiKey, forHTTPHeaderField: "x-cowchat-key")
        request.timeoutInterval = 60
        guard let (data, response) = try? await Self.thumbnailSession.data(for: request),
              (response as? HTTPURLResponse)?.statusCode == 200,
              let image = NSImage(data: data) else { return nil }
        if Self.thumbnailImages.count >= 200 { Self.thumbnailImages.removeAll() }
        Self.thumbnailImages[attachment.blobID] = image
        return image
    }

    /// Saves a picture attachment where the attachment chip saves files, then opens it in Quick Look, as pressing its
    /// thumbnail does.
    @discardableResult
    func openAttachment(_ attachment: FileAttachment) async throws -> URL {
        let saved = try await downloadAttachment(attachment)
        AttachmentPreview.show(saved)
        return saved
    }
}
