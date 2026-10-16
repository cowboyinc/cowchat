import AppKit
import SwiftUI
import XCTest
@testable import CowchatMac

final class ChartMessageTests: XCTestCase {
    private func fixture() throws -> String {
        let url = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
            .appendingPathComponent("../../../../test-fixtures/chart-v1.txt")
        return try String(contentsOf: url)
    }

    func testPortableChartEnvelopeDecodesARealPNG() throws {
        let chart = try XCTUnwrap(ChartMessage(content: fixture()))
        XCTAssertEqual(chart.title, "Example chart")
        XCTAssertNotNil(chart.image)
    }

    func testInvalidEnvelopesAndPixelBombsAreRejected() throws {
        XCTAssertNil(ChartMessage(content: "ordinary message"))
        XCTAssertNil(ChartMessage(content: ChartMessage.prefix + "{\"title\":\"x\",\"png\":\"not base64\"}"))
        let fixture = try XCTUnwrap(ChartMessage(content: fixture()))
        var png = fixture.png
        png.replaceSubrange(16..<20, with: [255, 255, 255, 255])
        let data = try JSONSerialization.data(withJSONObject: ["title": "bad dimensions", "png": png.base64EncodedString()])
        XCTAssertNil(ChartMessage(content: ChartMessage.prefix + String(decoding: data, as: UTF8.self)))
        XCTAssertNil(ChartMessage(content: String(repeating: "x", count: 400000)))
    }

    func testUnicodeChartTitlesAllowEmojiJoiners() throws {
        let chart = try XCTUnwrap(ChartMessage(content: fixture()))
        let payload = try JSONSerialization.data(withJSONObject: ["title": "👩‍💻 Growth", "png": chart.png.base64EncodedString()])
        XCTAssertEqual(ChartMessage(content: ChartMessage.prefix + String(decoding: payload, as: UTF8.self))?.title, "👩‍💻 Growth")
    }

    @MainActor
    func testChartRendersInsideAMessage() throws {
        let content = try fixture()
        let host = NSHostingView(rootView: ExpandableMessageText(content: content).padding(20).frame(width: 600, height: 360))
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 600, height: 360), styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        defer { window.close() }
        window.contentView = host
        window.display()
        host.layoutSubtreeIfNeeded()
        let bitmap = try XCTUnwrap(host.bitmapImageRepForCachingDisplay(in: host.bounds))
        host.cacheDisplay(in: host.bounds, to: bitmap)
        let png = try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
        XCTAssertGreaterThan(png.count, 1000)
        try png.write(to: URL(fileURLWithPath: "/tmp/cowchat-inline-chart-preview.png"))
    }
}

extension ChartMessageTests {
    @MainActor
    func testBundledOpenUIViewerRendersOfflineAndRejectsNavigation() async throws {
        let source = """
        root = Stack([chart])
        chart = BarChart(["Jan", "Feb"], [s1, s2], "grouped")
        s1 = Series("Revenue", [120, 150])
        s2 = Series("Cost", [90, 100])
        """
        var failure: String?
        let coordinator = InteractiveChartView.Coordinator(source: source, error: Binding(get: { failure }, set: { failure = $0 }))
        let web = InteractiveChartView.makeWebView(coordinator: coordinator)
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 960, height: 640), styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = web
        window.orderFront(nil)
        defer { web.stopLoading(); window.close() }
        var rendered = false
        for _ in 0..<100 {
            if failure != nil { break }
            if let result = try? await web.evaluateJavaScript("document.querySelectorAll('#chart svg path').length > 0"), result as? Bool == true {
                rendered = true
                break
            }
            try await Task.sleep(nanoseconds: 100_000_000)
        }
        XCTAssertNil(failure)
        XCTAssertTrue(rendered, "Bundled OpenUI must render in WebKit, not only Chromium")
        guard rendered else { return }
        let text = try await web.evaluateJavaScript("document.body.textContent") as? String ?? ""
        XCTAssertTrue(text.contains("Revenue"))
        let clicked = try await web.evaluateJavaScript("const item = document.querySelector('[class$=chart-legend-item]'); if (item) item.click(); !!item") as? Bool
        XCTAssertEqual(clicked, true)
        try await Task.sleep(nanoseconds: 200_000_000)
        let opacity = try await web.evaluateJavaScript("document.querySelector('[class$=chart-legend-item]').style.opacity") as? String
        XCTAssertEqual(opacity, "0.3", "Legend clicks must toggle the series in the native viewer")
        let policy = try await web.evaluateJavaScript("document.querySelector('meta[http-equiv=\"Content-Security-Policy\"]').content") as? String ?? ""
        XCTAssertTrue(policy.contains("connect-src 'none'"))
        _ = try await web.evaluateJavaScript("location.href = 'https://example.invalid/'")
        try await Task.sleep(nanoseconds: 100_000_000)
        XCTAssertEqual(web.url?.scheme, "cowchat-chart")
        let snapshot = try await web.takeSnapshot(configuration: nil)
        let bitmap = try XCTUnwrap(NSBitmapImageRep(data: try XCTUnwrap(snapshot.tiffRepresentation)))
        try XCTUnwrap(bitmap.representation(using: .png, properties: [:])).write(to: URL(fileURLWithPath: "/tmp/cowchat-interactive-chart-preview.png"))
    }
}
