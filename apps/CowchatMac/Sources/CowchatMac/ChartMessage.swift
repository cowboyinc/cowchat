import AppKit
import ImageIO
import SwiftUI

/// An image is part of normal message content, so room encryption protects it
/// and local rooms do not need a separate HTTP attachment server.
struct ChartMessage {
    static let prefix = "cowchat-chart:v1\n"
    static let maxPNGBytes = 256 * 1024
    let title: String
    let openui: String?
    let png: Data

    init?(content: String) {
        guard content.hasPrefix(Self.prefix),
              content.utf8.count <= Self.maxPNGBytes * 4 / 3 + 132 * 1024,
              let data = String(content.dropFirst(Self.prefix.count)).data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: String],
              Set(object.keys).isSubset(of: ["title", "png", "openui"]),
              let title = object["title"], !title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              title.unicodeScalars.count <= 200,
              title.unicodeScalars.allSatisfy({ $0.value > 31 && !(127...159).contains($0.value) }),
              let encoded = object["png"], let png = Data(base64Encoded: encoded), png.base64EncodedString() == encoded,
              png.count >= 33, png.count <= Self.maxPNGBytes,
              Array(png.prefix(16)) == [137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82]
        else { return nil }
        let bytes = [UInt8](png)
        let width = bytes[16..<20].reduce(UInt64(0)) { ($0 << 8) | UInt64($1) }
        let height = bytes[20..<24].reduce(UInt64(0)) { ($0 << 8) | UInt64($1) }
        guard width > 0, height > 0, width <= 4096, height <= 4096, width * height <= 8_000_000 else { return nil }
        let openui = object["openui"]
        guard openui == nil || (openui!.utf8.count <= 64 * 1024 && openui!.unicodeScalars.allSatisfy {
            !CharacterSet.controlCharacters.contains($0) || [9, 10, 13].contains($0.value)
        }) else { return nil }
        self.openui = openui
        self.title = title
        self.png = png
    }

    var image: NSImage? {
        guard let source = CGImageSourceCreateWithData(png as CFData, [kCGImageSourceShouldCache: false] as CFDictionary),
              let image = CGImageSourceCreateThumbnailAtIndex(source, 0, [
                kCGImageSourceCreateThumbnailFromImageAlways: true,
                kCGImageSourceThumbnailMaxPixelSize: 2048,
                kCGImageSourceShouldCacheImmediately: true,
              ] as CFDictionary)
        else { return nil }
        return NSImage(cgImage: image, size: .zero)
    }
}

struct ChartMessageView: View {
    let chart: ChartMessage
    @State private var exploring = false

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(chart.title).gallopText(.bodyMStrong, color: SemanticColor.textPrimary)
            if chart.openui != nil {
                Button("Explore chart", systemImage: "chart.xyaxis.line") { exploring = true }
                    .sheet(isPresented: $exploring) { InteractiveChartSheet(chart: chart) }
            }
            if let image = chart.image {
                Image(nsImage: image)
                    .resizable().aspectRatio(contentMode: .fit)
                    .frame(maxWidth: 640, maxHeight: 480)
                    .clipShape(RoundedRectangle(cornerRadius: 10))
                    .accessibilityLabel(chart.title)
                    .contextMenu {
                        Button("Save chart…") {
                            let panel = NSSavePanel()
                            panel.nameFieldStringValue = "chart.png"
                            panel.allowedContentTypes = [.png]
                            panel.begin { result in
                                if result == .OK, let url = panel.url {
                                    do { try chart.png.write(to: url) }
                                    catch { NSAlert(error: error).runModal() }
                                }
                            }
                        }
                    }
            } else {
                Label("This chart image could not be decoded.", systemImage: "photo")
                    .gallopText(.bodyS, color: SemanticColor.textTertiary)
            }
        }
    }
}
