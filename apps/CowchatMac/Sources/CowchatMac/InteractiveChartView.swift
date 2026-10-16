import SwiftUI
import WebKit

struct InteractiveChartSheet: View {
    let chart: ChartMessage
    @Environment(\.dismiss) private var dismiss
    @State private var error: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack {
                Text(chart.title).font(.headline)
                Spacer()
                Button("Done") { dismiss() }.keyboardShortcut(.cancelAction)
            }
            if let error {
                Text(error).foregroundStyle(.secondary)
                if let image = chart.image {
                    Image(nsImage: image).resizable().aspectRatio(contentMode: .fit)
                }
            } else {
                InteractiveChartView(source: chart.openui ?? "", error: $error)
            }
        }
        .padding(24).frame(width: 1000, height: 700)
    }
}

/// Every resource is bundled. Specs have no network, navigation, or native action capability.
struct InteractiveChartView: NSViewRepresentable {
    let source: String
    @Binding var error: String?

    func makeCoordinator() -> Coordinator { Coordinator(source: source, error: $error) }
    func makeNSView(context: Context) -> WKWebView {
        Self.makeWebView(coordinator: context.coordinator)
    }
    static func makeWebView(coordinator: Coordinator) -> WKWebView {
        let configuration = WKWebViewConfiguration()
        configuration.websiteDataStore = .nonPersistent()
        configuration.setURLSchemeHandler(coordinator, forURLScheme: "cowchat-chart")
        let webView = WKWebView(frame: .zero, configuration: configuration)
        webView.navigationDelegate = coordinator
        webView.load(URLRequest(url: URL(string: "cowchat-chart://bundle/index.html")!))
        return webView
    }
    func updateNSView(_ nsView: WKWebView, context: Context) {}
    static func dismantleNSView(_ nsView: WKWebView, coordinator: Coordinator) {
        nsView.stopLoading()
        nsView.navigationDelegate = nil
    }

    final class Coordinator: NSObject, WKURLSchemeHandler, WKNavigationDelegate {
        let source: String
        let error: Binding<String?>
        init(source: String, error: Binding<String?>) { self.source = source; self.error = error }

        func webView(_ webView: WKWebView, start urlSchemeTask: WKURLSchemeTask) {
            guard let url = urlSchemeTask.request.url, url.host == "bundle",
                  ["/index.html", "/client.js", "/client.css"].contains(url.path),
                  let resource = Bundle.module.url(forResource: String(url.path.dropFirst()), withExtension: nil, subdirectory: "Charts"),
                  let data = try? Data(contentsOf: resource)
            else { urlSchemeTask.didFailWithError(URLError(.resourceUnavailable)); return }
            let mime = url.path.hasSuffix(".js") ? "application/javascript" : url.path.hasSuffix(".css") ? "text/css" : "text/html"
            urlSchemeTask.didReceive(URLResponse(url: url, mimeType: mime, expectedContentLength: data.count, textEncodingName: "utf-8"))
            urlSchemeTask.didReceive(data)
            urlSchemeTask.didFinish()
        }
        func webView(_ webView: WKWebView, stop urlSchemeTask: WKURLSchemeTask) {}
        func webView(_ webView: WKWebView, decidePolicyFor navigationAction: WKNavigationAction,
                     decisionHandler: @escaping (WKNavigationActionPolicy) -> Void) {
            decisionHandler(navigationAction.request.url?.absoluteString == "cowchat-chart://bundle/index.html" ? .allow : .cancel)
        }
        func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
            Task { @MainActor [weak self] in
                guard let self else { return }
                do {
                    let value = try await webView.callAsyncJavaScript("window.renderChart(spec); return window.chartError;",
                        arguments: ["spec": source], in: nil, contentWorld: .page)
                    if let message = value as? String { error.wrappedValue = "Interactive chart unavailable: \(message)" }
                } catch {
                    self.error.wrappedValue = "Interactive chart unavailable. Showing the saved image."
                }
            }
        }
        func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) {
            self.error.wrappedValue = "Interactive chart unavailable. Showing the saved image."
        }
        func webViewWebContentProcessDidTerminate(_ webView: WKWebView) {
            error.wrappedValue = "Interactive chart stopped. Showing the saved image."
        }
    }
}
