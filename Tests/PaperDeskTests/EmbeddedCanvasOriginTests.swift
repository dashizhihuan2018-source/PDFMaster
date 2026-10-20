import AppKit
import WebKit
import XCTest

private final class ProbeScheme: NSObject, WKURLSchemeHandler {
    func webView(_ webView: WKWebView, start task: WKURLSchemeTask) {
        let html = "<html><body>local canvas<script>localStorage.setItem('probe','yes'); indexedDB.open('probe').onsuccess=()=>document.title='storage-ok';</script></body></html>"
        task.didReceive(URLResponse(url: task.request.url!, mimeType: "text/html", expectedContentLength: html.utf8.count, textEncodingName: "utf-8"))
        task.didReceive(Data(html.utf8)); task.didFinish()
    }
    func webView(_ webView: WKWebView, stop task: WKURLSchemeTask) {}
}

final class EmbeddedCanvasOriginTests: XCTestCase {
    @MainActor func testBundledOriginSupportsPersistentStorage() async throws {
        let config = WKWebViewConfiguration(); config.setURLSchemeHandler(ProbeScheme(), forURLScheme: "pdfmaster-canvas")
        let web = WKWebView(frame: CGRect(x: 0, y: 0, width: 900, height: 700), configuration: config)
        web.load(URLRequest(url: URL(string: "pdfmaster-canvas://workspace/canvas")!))
        for _ in 0..<100 {
            if web.title == "storage-ok" { break }
            try await Task.sleep(for: .milliseconds(100))
        }
        XCTAssertEqual(web.title, "storage-ok")
        web.stopLoading()
    }
}
